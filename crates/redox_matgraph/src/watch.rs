//! Polling watcher for `.matgraph` files (MAT-02 step 6).
//!
//! Same approach as `redox_script::ScriptWatcher`: no OS notifications. [`MatGraphWatcher::poll`]
//! compares the modification time and size with the previous poll, and parses and validates the
//! file only when it changed. Rebuilding the variants is left to the caller.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::graph::{GraphError, MaterialGraph};

/// File state compared between polls: modification time and size in bytes.
type FileStamp = (SystemTime, u64);

/// Outcome of one changed file.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphReload {
    /// The file parsed and validated; the caller rebuilds the variants from this graph.
    Ready(MaterialGraph),
    /// The file is broken; the caller keeps the variants it already has.
    Failed(Vec<GraphError>),
}

/// Watches one `.matgraph` file on disk.
#[derive(Debug)]
pub struct MatGraphWatcher {
    path: PathBuf,
    last: Option<FileStamp>,
}

impl MatGraphWatcher {
    /// Starts watching `path`. The current file state counts as already loaded, so the first
    /// poll does not report it. A file that does not exist yet is reported when it appears.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let last = file_stamp(&path);
        Self { path, last }
    }

    /// Returns one [`GraphReload`] if the file changed since the last poll, otherwise nothing.
    ///
    /// A failed reload is reported once for that file state: the new state is remembered
    /// whether or not it parsed, so the next poll stays silent until the file changes again.
    pub fn poll(&mut self) -> Vec<GraphReload> {
        let Some(stamp) = file_stamp(&self.path) else {
            return Vec::new();
        };
        if self.last == Some(stamp) {
            return Vec::new();
        }
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        self.last = Some(stamp);
        vec![reload_from_text(&text)]
    }
}

/// Parses and validates the text of a `.matgraph` file.
fn reload_from_text(text: &str) -> GraphReload {
    let graph = match MaterialGraph::from_ron(text) {
        Ok(graph) => graph,
        Err(error) => return GraphReload::Failed(vec![error]),
    };
    let errors = graph.validate();
    if errors.is_empty() {
        GraphReload::Ready(graph)
    } else {
        GraphReload::Failed(errors)
    }
}

/// Modification time and size of `path`, or `None` if the file cannot be inspected.
fn file_stamp(path: &Path) -> Option<FileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    Some((modified, meta.len()))
}
