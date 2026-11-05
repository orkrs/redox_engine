//! A temporary directory of its own, without external crates (EDT-A7).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Attempts to remove the directory in `Drop`. Windows may keep a file open for a moment.
const REMOVE_ATTEMPTS: usize = 5;
const REMOVE_RETRY: Duration = Duration::from_millis(20);

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// A unique directory under `std::env::temp_dir()`, removed when dropped.
#[derive(Debug)]
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Creates a new empty directory whose name contains `tag`, the process id and a counter.
    pub fn new(tag: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "redox_editor_a7_{tag}_{}_{stamp}_{id}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("the temporary directory is created");
        Self { path }
    }

    /// The directory itself.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        for _ in 0..REMOVE_ATTEMPTS {
            if std::fs::remove_dir_all(&self.path).is_ok() || !self.path.exists() {
                return;
            }
            std::thread::sleep(REMOVE_RETRY);
        }
    }
}

/// Lists the files under `dir`. Not recursive unless `recursive` is set. A missing directory
/// gives an empty list.
pub fn list_files(dir: &Path, recursive: bool) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if recursive {
                found.extend(list_files(&path, true));
            }
        } else {
            found.push(path);
        }
    }
    found.sort();
    found
}
