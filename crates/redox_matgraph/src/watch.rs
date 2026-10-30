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

/// Reads, parses and validates a `.matgraph` file once, for the first load of a binding.
///
/// An unreadable file gives one error without a node; parse and validation errors come as
/// [`reload_from_text`] reports them.
pub fn load_graph_file(path: &Path) -> Result<MaterialGraph, Vec<GraphError>> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        vec![GraphError::new(
            None,
            format!("не удалось прочитать {}: {error}", path.display()),
        )]
    })?;
    match reload_from_text(&text) {
        GraphReload::Ready(graph) => Ok(graph),
        GraphReload::Failed(errors) => Err(errors),
    }
}

/// Modification time and size of `path`, or `None` if the file cannot be inspected.
fn file_stamp(path: &Path) -> Option<FileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    Some((modified, meta.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Node, NodeKind, PortRef};

    /// Constant colour output, as the RON text of a valid graph.
    fn valid_text() -> String {
        MaterialGraph {
            version: 1,
            name: "colour".into(),
            nodes: vec![
                Node {
                    id: 1,
                    kind: NodeKind::ConstVec3([0.2, 0.4, 0.6]),
                    inputs: vec![],
                },
                Node {
                    id: 2,
                    kind: NodeKind::SurfaceOutput,
                    inputs: vec![
                        Some(PortRef { node: 1, port: 0 }),
                        None,
                        None,
                        None,
                        None,
                        None,
                    ],
                },
            ],
        }
        .to_ron()
    }

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{name}_{}.matgraph", std::process::id()))
    }

    #[test]
    fn a_valid_file_loads_into_its_graph() {
        let path = temp_file("load_valid");
        std::fs::write(&path, valid_text()).unwrap();
        let loaded = load_graph_file(&path);
        let _ = std::fs::remove_file(&path);
        let graph = loaded.expect("valid file loads");
        assert_eq!(graph.name, "colour");
    }

    #[test]
    fn a_missing_file_is_one_error_without_a_node() {
        let path = temp_file("load_missing_never_written");
        let errors = load_graph_file(&path).expect_err("missing file fails");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].node.is_none());
    }

    #[test]
    fn a_broken_file_reports_its_errors() {
        let path = temp_file("load_broken");
        std::fs::write(&path, "(this is not a graph)").unwrap();
        let errors = load_graph_file(&path);
        let _ = std::fs::remove_file(&path);
        assert!(errors.is_err_and(|e| !e.is_empty()));
    }
}
