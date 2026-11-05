//! Hot-reload and file-system observation for asset files.
//!
//! Uses the `notify` crate. The runtime asset manager reads
//! [`HotReloadWatcher::drain_reload_requests`] (modified files only). The editor reads
//! [`HotReloadWatcher::drain_events`] (create, modify, remove, rename) and applies them to the
//! asset registry. Sidecar (`.meta`) and temporary/backup files are never reported.

use crate::guid::is_sidecar_or_temp;
use log::trace;
use notify::event::{ModifyKind, RenameMode};
use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// Message sent when a watched path has changed.
#[derive(Debug, Clone)]
pub struct ReloadRequest {
    /// Path of the file that changed.
    pub path: PathBuf,
}

/// A file-system change of a watched asset path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetEvent {
    /// A file or folder appeared.
    Created(PathBuf),
    /// A file's contents or metadata changed.
    Modified(PathBuf),
    /// A file or folder disappeared.
    Removed(PathBuf),
    /// A file or folder was renamed or moved; both paths are known.
    Renamed {
        /// Path before the change.
        from: PathBuf,
        /// Path after the change.
        to: PathBuf,
    },
}

/// Watcher that reports file changes under the watched paths.
pub struct HotReloadWatcher {
    _watcher: RecommendedWatcher,
    receiver: mpsc::Receiver<notify::Result<notify::Event>>,
}

impl HotReloadWatcher {
    /// Creates a new watcher. Call [`watch`](Self::watch) to add paths.
    pub fn new() -> notify::Result<Self> {
        let (tx, receiver) = mpsc::channel();
        let watcher = RecommendedWatcher::new(
            move |res| {
                let _ = tx.send(res);
            },
            Config::default(),
        )?;
        Ok(Self {
            _watcher: watcher,
            receiver,
        })
    }

    /// Starts watching the given path (file or directory).
    pub fn watch(&mut self, path: impl AsRef<Path>) -> notify::Result<()> {
        self._watcher.watch(path.as_ref(), RecursiveMode::Recursive)
    }

    /// Stops watching the given path.
    pub fn unwatch(&mut self, path: impl AsRef<Path>) -> notify::Result<()> {
        self._watcher.unwatch(path.as_ref())
    }

    /// Drains pending events. Sidecar and temporary files are dropped and consecutive
    /// duplicates are merged.
    pub fn drain_events(&self) -> Vec<AssetEvent> {
        let mut out: Vec<AssetEvent> = Vec::new();
        while let Ok(res) = self.receiver.try_recv() {
            match res {
                Ok(ev) => {
                    for event in classify(ev) {
                        if out.last() != Some(&event) {
                            trace!("hot_reload: {event:?}");
                            out.push(event);
                        }
                    }
                }
                Err(e) => {
                    log::warn!("notify error: {e:?}");
                }
            }
        }
        out
    }

    /// Drains pending file-change events and returns paths that should be reloaded.
    /// Only modified files are returned; use [`drain_events`](Self::drain_events) for the rest.
    pub fn drain_reload_requests(&self) -> Vec<ReloadRequest> {
        self.drain_events()
            .into_iter()
            .filter_map(|event| match event {
                AssetEvent::Modified(path) => Some(ReloadRequest { path }),
                _ => None,
            })
            .collect()
    }
}

/// Maps one notify event to asset events and drops sidecar and temporary paths.
fn classify(ev: notify::Event) -> Vec<AssetEvent> {
    let paths = ev.paths;
    let events = match ev.kind {
        EventKind::Create(_) => paths.into_iter().map(AssetEvent::Created).collect(),
        EventKind::Remove(_) => paths.into_iter().map(AssetEvent::Removed).collect(),
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if paths.len() == 2 => {
            vec![AssetEvent::Renamed {
                from: paths[0].clone(),
                to: paths[1].clone(),
            }]
        }
        EventKind::Modify(ModifyKind::Name(_)) => paths
            .into_iter()
            .map(|p| {
                if p.exists() {
                    AssetEvent::Created(p)
                } else {
                    AssetEvent::Removed(p)
                }
            })
            .collect(),
        EventKind::Modify(_) => paths.into_iter().map(AssetEvent::Modified).collect(),
        _ => Vec::new(),
    };
    filter_sidecars(events)
}

/// Removes events about sidecar and temporary files. A rename from a temporary file counts as
/// a creation, a rename to one counts as a removal.
fn filter_sidecars(events: Vec<AssetEvent>) -> Vec<AssetEvent> {
    events
        .into_iter()
        .flat_map(|event| match event {
            AssetEvent::Renamed { from, to } => {
                match (is_sidecar_or_temp(&from), is_sidecar_or_temp(&to)) {
                    (false, false) => vec![AssetEvent::Renamed { from, to }],
                    (true, false) => vec![AssetEvent::Created(to)],
                    (false, true) => vec![AssetEvent::Removed(from)],
                    (true, true) => Vec::new(),
                }
            }
            other => {
                if is_sidecar_or_temp(primary_path(&other)) {
                    Vec::new()
                } else {
                    vec![other]
                }
            }
        })
        .collect()
}

fn primary_path(event: &AssetEvent) -> &Path {
    match event {
        AssetEvent::Created(p) | AssetEvent::Modified(p) | AssetEvent::Removed(p) => p.as_path(),
        AssetEvent::Renamed { from, .. } => from.as_path(),
    }
}
