//! Background import queue: worker threads run importers with priorities, cancellation and retry.
//! Each job runs under `catch_unwind`, so a failing or panicking import reports `Failed` and the
//! other jobs and the editor keep running. Source files are only read.

use std::any::Any;
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use redox_asset::AssetGuid;

use crate::import::{
    AssetImporter, CancelToken, ImportError, ImportJob, ImportReport, ImporterRegistry,
};

/// Upper bound of the default number of import workers.
pub const MAX_IMPORT_WORKERS: usize = 4;

/// Default worker count: `cores - 1`, clamped to `1..=MAX_IMPORT_WORKERS`.
pub fn default_worker_count() -> usize {
    std::thread::available_parallelism()
        .map_or(1, |n| n.get().saturating_sub(1))
        .clamp(1, MAX_IMPORT_WORKERS)
}

/// Events reported by the queue; the service drains them in its tick.
#[derive(Clone, Debug, PartialEq)]
pub enum ImportEvent {
    /// A worker started the job.
    Started {
        /// Absolute path of the source.
        path: PathBuf,
        /// Identity of the asset.
        guid: AssetGuid,
    },
    /// Progress in `0.0..=1.0`. Importers have no progress callback yet, so the queue does not emit this.
    Progress {
        /// Absolute path of the source.
        path: PathBuf,
        /// Fraction done.
        fraction: f32,
    },
    /// The job finished; `importer_version` is recorded in the sidecar.
    Finished {
        /// Absolute path of the source.
        path: PathBuf,
        /// Identity of the asset.
        guid: AssetGuid,
        /// Version of the importer that ran.
        importer_version: u32,
        /// Facts and warnings from the importer.
        report: ImportReport,
    },
    /// The job failed, or its importer panicked; the message explains why.
    Failed {
        /// Absolute path of the source.
        path: PathBuf,
        /// Identity of the asset.
        guid: AssetGuid,
        /// Explanation for the user.
        message: String,
    },
    /// The job was cancelled, before or during its run.
    Cancelled {
        /// Absolute path of the source.
        path: PathBuf,
        /// Identity of the asset.
        guid: AssetGuid,
    },
}

struct Pending {
    path: PathBuf,
    guid: AssetGuid,
    importer: Arc<dyn AssetImporter>,
    seq: u64,
}

#[derive(Default)]
struct State {
    pending: Vec<Pending>,
    active: HashMap<PathBuf, CancelToken>,
    known: HashMap<PathBuf, AssetGuid>,
    open: Option<PathBuf>,
    seq: u64,
    shutdown: bool,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

fn lock(shared: &Shared) -> MutexGuard<'_, State> {
    shared.state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Background import queue with a pool of worker threads.
pub struct ImportQueue {
    shared: Arc<Shared>,
    importers: ImporterRegistry,
    events_tx: flume::Sender<ImportEvent>,
    events_rx: flume::Receiver<ImportEvent>,
}

impl ImportQueue {
    /// Starts `workers` threads (at least one is requested; spawn failures are logged).
    pub fn new(importers: ImporterRegistry, workers: usize) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
        });
        let (events_tx, events_rx) = flume::unbounded();
        for index in 0..workers.max(1) {
            let worker_shared = Arc::clone(&shared);
            let worker_events = events_tx.clone();
            let spawned = std::thread::Builder::new()
                .name(format!("import-worker-{index}"))
                .spawn(move || worker_loop(&worker_shared, &worker_events)); // undo: n/a
            if let Err(err) = spawned {
                log::error!("import worker {index} was not started: {err}");
            }
        }
        Self {
            shared,
            importers,
            events_tx,
            events_rx,
        }
    }

    /// Queues an import of `path` with the importer for its extension. Returns `false` when no
    /// importer handles the file, or the file is already queued or running.
    pub fn submit(&self, path: PathBuf, guid: AssetGuid) -> bool {
        let Some(importer) = self.importers.for_path(&path) else {
            return false;
        };
        let mut state = lock(&self.shared);
        if state.shutdown
            || state.active.contains_key(&path)
            || state.pending.iter().any(|p| p.path == path)
        {
            return false;
        }
        state.seq += 1;
        let seq = state.seq;
        state.known.insert(path.clone(), guid);
        state.pending.push(Pending {
            path,
            guid,
            importer,
            seq,
        });
        drop(state);
        self.shared.wake.notify_one();
        true
    }

    /// Queues again a file whose last import failed or was cancelled.
    pub fn retry(&self, path: &Path) -> bool {
        let guid = lock(&self.shared).known.get(path).copied();
        match guid {
            Some(guid) => self.submit(path.to_path_buf(), guid),
            None => false,
        }
    }

    /// Cancels a queued job (it reports `Cancelled` at once) or asks a running one to stop.
    /// Returns `false` when the file is neither queued nor running.
    pub fn cancel(&self, path: &Path) -> bool {
        let mut state = lock(&self.shared);
        if let Some(index) = state.pending.iter().position(|p| p.path.as_path() == path) {
            let job = state.pending.remove(index);
            drop(state);
            let _ = self.events_tx.send(ImportEvent::Cancelled {
                path: job.path,
                guid: job.guid,
            });
            return true;
        }
        match state.active.get(path) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    /// Sets the asset open in the editor; its queued jobs run first.
    pub fn set_open(&self, path: Option<PathBuf>) {
        lock(&self.shared).open = path;
    }

    /// Takes the events produced since the last call.
    pub fn drain_events(&self) -> Vec<ImportEvent> {
        self.events_rx.try_iter().collect()
    }

    /// Number of jobs waiting for a worker.
    pub fn pending_count(&self) -> usize {
        lock(&self.shared).pending.len()
    }

    /// Number of jobs running now.
    pub fn active_count(&self) -> usize {
        lock(&self.shared).active.len()
    }
}

impl Drop for ImportQueue {
    fn drop(&mut self) {
        lock(&self.shared).shutdown = true;
        self.shared.wake.notify_all();
        // Workers finish their current job and exit. They are detached rather than joined, so
        // closing the editor never waits for a long import.
    }
}

fn worker_loop(shared: &Shared, events: &flume::Sender<ImportEvent>) {
    while let Some((job, token)) = next_job(shared) {
        run_job(shared, events, job, &token);
    }
}

/// Waits for a job; `None` once the queue is shut down.
fn next_job(shared: &Shared) -> Option<(Pending, CancelToken)> {
    let mut state = lock(shared);
    loop {
        if state.shutdown {
            return None;
        }
        if let Some(index) = pick(&state) {
            let job = state.pending.remove(index);
            let token = CancelToken::new();
            state.active.insert(job.path.clone(), token.clone());
            return Some((job, token));
        }
        state = shared
            .wake
            .wait(state)
            .unwrap_or_else(PoisonError::into_inner);
    }
}

/// Index of the next job: the open asset first, then first come, first served.
fn pick(state: &State) -> Option<usize> {
    state
        .pending
        .iter()
        .enumerate()
        .max_by_key(|(_, job)| {
            (
                state.open.as_deref() == Some(job.path.as_path()),
                std::cmp::Reverse(job.seq),
            )
        })
        .map(|(index, _)| index)
}

fn run_job(
    shared: &Shared,
    events: &flume::Sender<ImportEvent>,
    job: Pending,
    token: &CancelToken,
) {
    let Pending {
        path,
        guid,
        importer,
        seq: _,
    } = job;
    let _ = events.send(ImportEvent::Started {
        path: path.clone(),
        guid,
    });
    let request = ImportJob {
        path: path.clone(),
        guid,
        settings: String::new(),
    };
    let outcome = catch_unwind(AssertUnwindSafe(|| importer.import(&request, token)));
    let event = match outcome {
        Ok(Ok(output)) => ImportEvent::Finished {
            path: path.clone(),
            guid,
            importer_version: importer.version(),
            report: output.report,
        },
        Ok(Err(ImportError::Cancelled)) => ImportEvent::Cancelled {
            path: path.clone(),
            guid,
        },
        Ok(Err(error)) => {
            log::error!("import of {} failed: {error}", path.display());
            ImportEvent::Failed {
                path: path.clone(),
                guid,
                message: error.to_string(),
            }
        }
        Err(payload) => {
            let message = format!(
                "importer {} panicked: {}",
                importer.name(),
                panic_message(payload.as_ref())
            );
            log::error!("import of {} {message}", path.display());
            ImportEvent::Failed {
                path: path.clone(),
                guid,
                message,
            }
        }
    };
    lock(shared).active.remove(&path);
    let _ = events.send(event);
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "unknown panic".to_owned()
    }
}
