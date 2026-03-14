//! Hot-reload: watch asset files and re-load when they change.
//!
//! Uses the `notify` crate to observe the filesystem. When a watched file
//! is modified, the path is sent to a channel; the asset manager drains
//! this channel and re-queues the load for that path.

use std::path::Path;
use std::sync::mpsc;
use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use log::trace;

/// Message sent when a watched path has changed.
#[derive(Debug, Clone)]
pub struct ReloadRequest {
    pub path: std::path::PathBuf,
}

/// Watcher that emits [`ReloadRequest`] on file changes.
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
        self._watcher
            .watch(path.as_ref(), RecursiveMode::Recursive)
    }

    /// Stops watching the given path.
    pub fn unwatch(&mut self, path: impl AsRef<Path>) -> notify::Result<()> {
        self._watcher.unwatch(path.as_ref())
    }

    /// Drains pending file-change events and returns paths that should be reloaded.
    pub fn drain_reload_requests(&self) -> Vec<ReloadRequest> {
        let mut out = Vec::new();
        while let Ok(res) = self.receiver.try_recv() {
            match res {
                Ok(ev) => {
                    if let EventKind::Modify(_) = ev.kind {
                        for path in ev.paths {
                            trace!("hot_reload: {:?} modified", path);
                            out.push(ReloadRequest { path });
                        }
                    }
                }
                Err(e) => {
                    log::warn!("notify error: {:?}", e);
                }
            }
        }
        out
    }
}
