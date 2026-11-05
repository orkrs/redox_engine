//! Autosave: periodic snapshots of a dirty level in `<project>/.redox/autosave`.
//!
//! The level file itself is never touched. A snapshot is taken on the calling (main) thread as
//! bytes, then written as `<level>.<n>.scene.ron` through a temporary file and a rename, either on
//! a background thread ([`WriteMode::Background`]) or inline ([`WriteMode::Synchronous`], used by
//! tests). The service knows nothing about scenes: the editor supplies the level through
//! [`AutosaveSource`] and the time through [`Clock`].

use crate::log::EditorLog;
use crate::recovery::{SessionLock, remove_lock, write_lock};
use redox_scene::ofpa::write_atomic;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Directory of autosave snapshots, relative to the project root.
pub const AUTOSAVE_SUBDIR: &str = ".redox/autosave";
/// Smallest allowed autosave interval in seconds.
pub const MIN_INTERVAL_SECS: u32 = 10;
/// Largest allowed autosave interval in seconds.
pub const MAX_INTERVAL_SECS: u32 = 3600;
/// Longest wait for the background writer in [`AutosaveService::flush`].
pub const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);
/// Attempts of a write that fails, for example while Windows still holds the file.
pub const WRITE_ATTEMPTS: u32 = 3;
/// Pause between two attempts of a failed write.
pub const WRITE_RETRY_PAUSE: Duration = Duration::from_millis(50);
/// Seconds between heartbeat updates of `session.lock`.
pub const HEARTBEAT_SECS: u64 = 5;

/// When autosave runs and how many snapshots of a level are kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutosavePolicy {
    /// Seconds since the last autosave after which a dirty level is saved again.
    pub interval_secs: u32,
    /// Committed transactions after which a dirty level is saved again, whatever the time.
    pub every_n_transactions: u32,
    /// Number of snapshot files kept per level.
    pub keep: usize,
    /// Whether autosave runs at all.
    pub enabled: bool,
}

impl Default for AutosavePolicy {
    fn default() -> Self {
        Self {
            interval_secs: 120,
            every_n_transactions: 25,
            keep: 3,
            enabled: true,
        }
    }
}

impl AutosavePolicy {
    /// Copy with every numeric field brought into its allowed range.
    pub fn clamped(self) -> Self {
        Self {
            interval_secs: self.interval_secs.clamp(MIN_INTERVAL_SECS, MAX_INTERVAL_SECS),
            every_n_transactions: self.every_n_transactions.clamp(1, 10_000),
            keep: self.keep.clamp(1, 20),
            enabled: self.enabled,
        }
    }
}

/// What the autosave service needs from the level that is being edited.
pub trait AutosaveSource {
    /// Whether the level has changes that are not in its file.
    fn is_dirty(&self) -> bool;
    /// Name used for the snapshot file names; the service sanitizes it.
    fn level_name(&self) -> String;
    /// File of the level, or `None` for a level that was never saved.
    fn level_path(&self) -> Option<PathBuf>;
    /// The serialized level, exactly as it would be written to its file.
    fn snapshot_bytes(&self) -> Result<Vec<u8>, String>;
}

/// Source of whole seconds since the Unix epoch.
pub trait Clock {
    /// Seconds since the Unix epoch.
    fn now_secs(&self) -> u64;
}

/// The system clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_secs(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs())
    }
}

/// A clock that moves only when told to. Clones share the same time. Used by tests.
#[derive(Clone, Debug, Default)]
pub struct ManualClock {
    now: Arc<AtomicU64>,
}

impl ManualClock {
    /// Creates a clock that reads `start_secs`.
    pub fn new(start_secs: u64) -> Self {
        Self {
            now: Arc::new(AtomicU64::new(start_secs)),
        }
    }

    /// Sets the current time.
    pub fn set(&self, secs: u64) {
        self.now.store(secs, Ordering::SeqCst);
    }

    /// Moves the current time forward by `secs`.
    pub fn advance(&self, secs: u64) {
        self.now.fetch_add(secs, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_secs(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }
}

/// How snapshots reach the disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteMode {
    /// A background thread writes; the editor does not wait for the disk.
    Background,
    /// The write happens inside [`AutosaveService::tick`]. Used by tests.
    Synchronous,
}

/// Result of one [`AutosaveService::tick`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TickOutcome {
    /// Nothing to do: autosave is off, the level is clean, or no trigger has fired yet.
    Idle,
    /// A snapshot was taken and handed to the writer; the path is where it goes.
    Queued(PathBuf),
}

/// Directory that holds the snapshots of a project.
pub fn autosave_dir(project_root: &Path) -> PathBuf {
    project_root.join(AUTOSAVE_SUBDIR)
}

/// File-name stem for a level name: every character except letters, digits, `-` and `_`
/// becomes `_`, and an empty name becomes `level`.
pub fn file_stem_for(level: &str) -> String {
    let stem: String = level
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if stem.is_empty() {
        "level".to_string()
    } else {
        stem
    }
}

/// Snapshot files of a level in `dir`, as `(number, path)` sorted by number, oldest first.
pub fn list_autosaves(dir: &Path, stem: &str) -> Vec<(usize, PathBuf)> {
    let prefix = format!("{stem}.");
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(usize, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            let name = path.file_name()?.to_str()?;
            let middle = name
                .strip_prefix(prefix.as_str())?
                .strip_suffix(".scene.ron")?;
            let number = middle.parse::<usize>().ok()?;
            Some((number, path))
        })
        .collect();
    found.sort_by_key(|(number, _)| *number);
    found
}

/// Writes `bytes` to `path` atomically, retrying a failed attempt up to [`WRITE_ATTEMPTS`] times.
fn write_with_retry(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut last_error = None;
    for attempt in 0..WRITE_ATTEMPTS {
        if attempt > 0 {
            thread::sleep(WRITE_RETRY_PAUSE);
        }
        match write_atomic(path, bytes) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| io::Error::other("write failed")))
}

/// Writes one snapshot and then removes the `prune` files. Returns whether the write succeeded;
/// a failure is logged and the next trigger tries again.
fn perform_write(log: &EditorLog, path: &Path, bytes: &[u8], prune: &[PathBuf]) -> bool {
    if let Err(err) = write_with_retry(path, bytes) {
        log.error(format!(
            "autosave write failed for {}: {err}; the next cycle retries",
            path.display()
        ));
        return false;
    }
    for old in prune {
        if let Err(err) = fs::remove_file(old)
            && err.kind() != io::ErrorKind::NotFound
        {
            log.warn(format!("could not remove old autosave {}: {err}", old.display()));
        }
    }
    true
}

/// Work for the background writer thread.
enum Job {
    /// Write `bytes` to `path`, then remove the `prune` files.
    Write {
        path: PathBuf,
        bytes: Vec<u8>,
        prune: Vec<PathBuf>,
    },
    /// Acknowledge through the channel once everything queued before it is written.
    Flush(flume::Sender<()>),
}

/// Where snapshots are written.
enum Writer {
    /// Written inline by the caller.
    Synchronous,
    /// Sent to the background thread.
    Background(flume::Sender<Job>),
}

/// Starts the background writer. Falls back to inline writes if the thread cannot start.
fn spawn_writer(log: &EditorLog) -> Writer {
    let (sender, receiver) = flume::unbounded::<Job>();
    let thread_log = log.clone();
    let spawned = thread::Builder::new()
        .name("redox-autosave".to_string())
        .spawn(move || {
            for job in receiver.iter() {
                match job {
                    Job::Write {
                        path,
                        bytes,
                        prune,
                    } => {
                        perform_write(&thread_log, &path, &bytes, &prune);
                    }
                    Job::Flush(ack) => {
                        let _ = ack.send(());
                    }
                }
            }
        });
    match spawned {
        Ok(_) => Writer::Background(sender),
        Err(err) => {
            log.error(format!(
                "autosave thread could not start, writing inline: {err}"
            ));
            Writer::Synchronous
        }
    }
}

/// Periodic autosave of the level that is being edited.
pub struct AutosaveService {
    project_root: PathBuf,
    policy: AutosavePolicy,
    clock: Box<dyn Clock>,
    log: EditorLog,
    writer: Writer,
    session: Option<SessionLock>,
    interval_anchor_secs: u64,
    transactions_anchor: u64,
    last_saved_secs: Option<u64>,
}

impl AutosaveService {
    /// Creates a service for the project at `project_root`. Nothing is written until a tick
    /// fires. The policy is clamped into its allowed range.
    pub fn new(
        project_root: impl Into<PathBuf>,
        policy: AutosavePolicy,
        clock: Box<dyn Clock>,
        log: EditorLog,
        mode: WriteMode,
    ) -> Self {
        let now = clock.now_secs();
        let writer = match mode {
            WriteMode::Synchronous => Writer::Synchronous,
            WriteMode::Background => spawn_writer(&log),
        };
        Self {
            project_root: project_root.into(),
            policy: policy.clamped(),
            clock,
            log,
            writer,
            session: None,
            interval_anchor_secs: now,
            transactions_anchor: 0,
            last_saved_secs: None,
        }
    }

    /// Current policy.
    pub fn policy(&self) -> AutosavePolicy {
        self.policy
    }

    /// Replaces the policy; the new values are clamped into their allowed range.
    pub fn set_policy(&mut self, policy: AutosavePolicy) {
        self.policy = policy.clamped();
    }

    /// Project root that holds `.redox/`.
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// Time of the last snapshot handed to the writer, if any.
    pub fn last_saved_secs(&self) -> Option<u64> {
        self.last_saved_secs
    }

    /// Runs one autosave cycle. `transactions` is the number of committed transactions so far;
    /// the service compares it with the value at its last snapshot. A snapshot is taken when the
    /// level is dirty and either the interval has passed or enough transactions were committed.
    pub fn tick(&mut self, source: &dyn AutosaveSource, transactions: u64) -> TickOutcome {
        let now = self.clock.now_secs();
        self.refresh_heartbeat(now);
        if !self.policy.enabled || !source.is_dirty() {
            return TickOutcome::Idle;
        }
        if !self.trigger_due(now, transactions) {
            return TickOutcome::Idle;
        }
        self.snapshot(source, now, transactions)
    }

    /// Whether this service holds the session lock of its project.
    pub fn is_session_active(&self) -> bool {
        self.session.is_some()
    }

    /// Takes the session lock of the project for `source`. Call once the recovery question is
    /// settled, so that a stale lock is never overwritten before the user answered.
    pub fn begin_session(&mut self, source: &dyn AutosaveSource) -> io::Result<()> {
        let now = self.clock.now_secs();
        let lock = SessionLock {
            pid: std::process::id(),
            started: now,
            project: self.project_root.display().to_string(),
            level: source.level_name(),
            heartbeat: now,
        };
        write_lock(&self.project_root, &lock)?;
        self.session = Some(lock);
        self.interval_anchor_secs = now;
        Ok(())
    }

    /// Ends the session at a normal close. Flushes the writer first. When the level is clean
    /// (`dirty == false`), the lock and the autosaves of the level are removed. When it has
    /// unsaved changes, both stay so that the next start can offer recovery.
    pub fn finish_session(&mut self, dirty: bool) -> io::Result<()> {
        self.flush();
        let Some(lock) = self.session.take() else {
            return Ok(());
        };
        if dirty {
            self.log.info(format!(
                "{} closed with unsaved changes; autosaves are kept for recovery",
                lock.level
            ));
            return Ok(());
        }
        remove_lock(&self.project_root)?;
        let dir = autosave_dir(&self.project_root);
        for (_, path) in list_autosaves(&dir, &file_stem_for(&lock.level)) {
            if let Err(err) = fs::remove_file(&path) {
                self.log.warn(format!("could not remove {}: {err}", path.display()));
            }
        }
        Ok(())
    }

    /// Moves the heartbeat of the session lock forward every [`HEARTBEAT_SECS`] seconds.
    fn refresh_heartbeat(&mut self, now: u64) {
        let Some(lock) = self.session.as_mut() else {
            return;
        };
        if now.saturating_sub(lock.heartbeat) < HEARTBEAT_SECS {
            return;
        }
        lock.heartbeat = now;
        if let Err(err) = write_lock(&self.project_root, lock) {
            self.log.warn(format!("session heartbeat was not written: {err}"));
        }
    }

    /// Waits until the background writer has written everything queued so far. Returns `false`
    /// if that did not happen within [`FLUSH_TIMEOUT`].
    pub fn flush(&self) -> bool {
        match &self.writer {
            Writer::Synchronous => true,
            Writer::Background(sender) => {
                let (ack_tx, ack_rx) = flume::bounded::<()>(1);
                if sender.send(Job::Flush(ack_tx)).is_err() {
                    return false;
                }
                match ack_rx.recv_timeout(FLUSH_TIMEOUT) {
                    Ok(()) => true,
                    Err(_) => {
                        self.log.warn("autosave queue did not drain in time");
                        false
                    }
                }
            }
        }
    }

    /// Whether the interval or the transaction count says a snapshot is due.
    fn trigger_due(&self, now: u64, transactions: u64) -> bool {
        let by_time = now.saturating_sub(self.interval_anchor_secs)
            >= u64::from(self.policy.interval_secs);
        let by_count = transactions.saturating_sub(self.transactions_anchor)
            >= u64::from(self.policy.every_n_transactions);
        by_time || by_count
    }

    /// Takes the snapshot, picks the next file number and hands the write to the writer.
    fn snapshot(&mut self, source: &dyn AutosaveSource, now: u64, transactions: u64) -> TickOutcome {
        // Whatever happens below, the next attempt waits for the next trigger.
        self.interval_anchor_secs = now;
        self.transactions_anchor = transactions;
        let bytes = match source.snapshot_bytes() {
            Ok(bytes) => bytes,
            Err(err) => {
                self.log.error(format!("autosave snapshot failed: {err}"));
                return TickOutcome::Idle;
            }
        };
        let dir = autosave_dir(&self.project_root);
        let stem = file_stem_for(&source.level_name());
        let existing = list_autosaves(&dir, &stem);
        let number = existing.last().map_or(1, |(last, _)| *last + 1);
        let path = dir.join(format!("{stem}.{number}.scene.ron"));
        let keep = self.policy.keep.max(1);
        let remove = (existing.len() + 1).saturating_sub(keep);
        let prune: Vec<PathBuf> = existing
            .iter()
            .take(remove)
            .map(|(_, old)| old.clone())
            .collect();
        match &self.writer {
            Writer::Synchronous => {
                if !perform_write(&self.log, &path, &bytes, &prune) {
                    return TickOutcome::Idle;
                }
            }
            Writer::Background(sender) => {
                let job = Job::Write {
                    path: path.clone(),
                    bytes,
                    prune,
                };
                if sender.send(job).is_err() {
                    self.log.error("autosave writer has stopped");
                    return TickOutcome::Idle;
                }
            }
        }
        self.last_saved_secs = Some(now);
        TickOutcome::Queued(path)
    }
}
