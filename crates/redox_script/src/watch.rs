//! Polling watcher that reloads a script instance when its `.wasm` file changes on disk.
//!
//! No file-system notifications are used. [`ScriptWatcher::poll`] compares the modification time
//! and size of the file with the values seen at the previous poll. Call it from the game loop
//! about every 0.5 s; it does a `metadata` call each time and reads the file only when it changed.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::{InstanceId, ScriptRuntime};

/// The file state compared between polls: modification time and size in bytes.
type FileStamp = (SystemTime, u64);

/// Watches one `.wasm` file on disk and reloads the script instance when the file changes.
#[derive(Debug)]
pub struct ScriptWatcher {
    path: PathBuf,
    id: InstanceId,
    last: Option<FileStamp>,
}

/// What [`ScriptWatcher::poll`] found.
#[derive(Clone, Debug, PartialEq)]
pub enum WatchEvent {
    /// The file is unchanged since the last poll (or does not exist yet and did not before).
    Unchanged,
    /// The file changed and the instance was reloaded.
    Reloaded,
    /// The file changed but reloading failed; the old module keeps running. Holds the error text.
    Failed(String),
}

impl ScriptWatcher {
    /// Starts watching `path` for `id`.
    ///
    /// The current state of the file is taken as "already loaded", so the first poll does not
    /// reload. A file that does not exist yet is reloaded by the first poll after it appears.
    pub fn new(path: impl Into<PathBuf>, id: InstanceId) -> Self {
        let path = path.into();
        let last = file_stamp(&path);
        Self { path, id, last }
    }

    /// Looks at the file and, if its modification time or size differs from the last poll, reads
    /// it and calls [`ScriptRuntime::reload`] for the watched instance.
    ///
    /// A missing or unreadable file is [`WatchEvent::Unchanged`] and the last seen state is kept:
    /// editors often save by delete-and-rename, and the next poll sees the new file. A failed
    /// reload is reported once for that file state, not on every poll, and the old module keeps
    /// running.
    pub fn poll(&mut self, runtime: &mut ScriptRuntime) -> WatchEvent {
        let Some(stamp) = file_stamp(&self.path) else {
            return WatchEvent::Unchanged;
        };
        if self.last == Some(stamp) {
            return WatchEvent::Unchanged;
        }
        let Ok(bytes) = std::fs::read(&self.path) else {
            return WatchEvent::Unchanged;
        };
        self.last = Some(stamp);
        match runtime.reload(self.id, &bytes) {
            Ok(()) => WatchEvent::Reloaded,
            Err(error) => WatchEvent::Failed(format!("{error:?}")),
        }
    }
}

/// Modification time and size of `path`, or `None` if the file cannot be inspected.
fn file_stamp(path: &Path) -> Option<FileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    Some((modified, meta.len()))
}
