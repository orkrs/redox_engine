//! Material graph document: open and save `.matgraph` files (RON), dirty tracking and migrations.
//!
//! Per game spec (ТЗ §7.3), a graph that fails typecheck or WGSL emit is not written to disk.

use std::fs;
use std::path::{Path, PathBuf};

use redox_asset::AssetGuid;
use redox_matgraph::graph::FORMAT_VERSION;
use redox_matgraph::{CodegenOptions, Features, GraphError, MaterialGraph, Pass, compile_variant};
use redox_scene::write_atomic;

/// Errors when opening or saving a material document.
#[derive(Debug, thiserror::Error)]
pub enum MaterialError {
    /// The file could not be read or written.
    #[error("cannot access {}: {source}", .path.display())]
    Io {
        /// File involved.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The file is not valid RON for a material graph.
    #[error("cannot parse {}: {message}", .path.display())]
    Parse {
        /// File that failed.
        path: PathBuf,
        /// Parser message.
        message: String,
    },
    /// The file was written by a newer editor.
    #[error("matgraph format {found} is newer than supported {supported}")]
    NewerFormat {
        /// Version in the file.
        found: u32,
        /// Highest version this build writes.
        supported: u32,
    },
    /// Save was blocked because the graph does not compile for the game.
    #[error("material cannot be saved: {reason}")]
    SaveBlocked {
        /// Short summary.
        reason: String,
        /// Full validation or compile errors.
        errors: Vec<GraphError>,
    },
    /// The document has no path yet.
    #[error("material has no file yet; use save_as")]
    NoPath,
}

/// Result of a successful compile used before save.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledMaterial {
    /// WGSL source of the surface pass.
    pub wgsl: String,
}

/// An open `.matgraph` document bound to an asset identity when known.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialDocument {
    /// Path on disk. `None` until the first save or open.
    pub path: Option<PathBuf>,
    /// Asset GUID when opened from the project registry.
    pub asset_guid: Option<AssetGuid>,
    /// The graph being edited.
    pub graph: MaterialGraph,
    /// Graph bytes last accepted by a successful save (dirty when different).
    saved_graph: MaterialGraph,
    /// Format version after load; saves always write [`FORMAT_VERSION`].
    pub format_version: u32,
    /// Previous on-disk version when migration ran.
    migrated_from: Option<u32>,
}

impl MaterialDocument {
    /// Empty untitled document.
    pub fn new(name: impl Into<String>) -> Self {
        let graph = MaterialGraph {
            version: FORMAT_VERSION,
            name: name.into(),
            nodes: Vec::new(),
        };
        Self {
            path: None,
            asset_guid: None,
            graph: graph.clone(),
            saved_graph: graph,
            format_version: FORMAT_VERSION,
            migrated_from: None,
        }
    }

    /// Whether the in-memory graph differs from the last successful save.
    pub fn is_dirty(&self) -> bool {
        self.graph != self.saved_graph
    }

    /// Opens a `.matgraph` file. The file is only read.
    pub fn open(
        path: impl AsRef<Path>,
        asset_guid: Option<AssetGuid>,
    ) -> Result<Self, MaterialError> {
        let path = path.as_ref();
        let text = fs::read_to_string(path).map_err(|source| io_error(path, source))?;
        let mut graph = MaterialGraph::from_ron(&text).map_err(|e| MaterialError::Parse {
            path: path.to_path_buf(),
            message: e.message,
        })?;
        let found = graph.version;
        if found > FORMAT_VERSION {
            return Err(MaterialError::NewerFormat {
                found,
                supported: FORMAT_VERSION,
            });
        }
        let migrated_from = migrate_graph(&mut graph, found);
        let saved = graph.clone();
        Ok(Self {
            path: Some(path.to_path_buf()),
            asset_guid,
            graph,
            saved_graph: saved,
            format_version: FORMAT_VERSION,
            migrated_from,
        })
    }

    /// Saves to the document path. Fails when the graph does not typecheck and emit.
    pub fn save(&mut self) -> Result<CompiledMaterial, MaterialError> {
        let path = self.path.clone().ok_or(MaterialError::NoPath)?;
        self.write_to(&path)
    }

    /// Saves to `path` and makes it the document path.
    pub fn save_as(&mut self, path: impl AsRef<Path>) -> Result<CompiledMaterial, MaterialError> {
        let path = path.as_ref();
        let compiled = self.write_to(path)?;
        self.path = Some(path.to_path_buf());
        Ok(compiled)
    }

    /// Checks and compiles the graph without writing a file.
    pub fn compile_check(&self) -> Result<CompiledMaterial, MaterialError> {
        compile_for_save(&self.graph)
    }

    fn write_to(&mut self, path: &Path) -> Result<CompiledMaterial, MaterialError> {
        let compiled = compile_for_save(&self.graph)?;
        self.backup_if_migrated(path)?;
        let bytes = self.graph.to_ron().into_bytes();
        write_atomic(path, &bytes).map_err(|source| io_error(path, source))?;
        self.saved_graph = self.graph.clone();
        self.format_version = FORMAT_VERSION;
        self.migrated_from = None;
        Ok(compiled)
    }

    fn backup_if_migrated(&self, path: &Path) -> Result<(), MaterialError> {
        if let Some(from) = self.migrated_from
            && path.exists()
        {
            let backup = backup_path(path, from);
            fs::copy(path, &backup).map_err(|source| io_error(&backup, source))?;
        }
        Ok(())
    }
}

/// Runs typecheck and WGSL emit the way the renderer expects for the main pass.
pub fn compile_for_save(graph: &MaterialGraph) -> Result<CompiledMaterial, MaterialError> {
    let errors = graph.validate();
    if !errors.is_empty() {
        return Err(MaterialError::SaveBlocked {
            reason: format!("{} validation error(s)", errors.len()),
            errors,
        });
    }
    let opts = CodegenOptions {
        pass: Pass::Main,
        features: Features::default(),
    };
    match compile_variant(graph, &opts) {
        Ok(v) => Ok(CompiledMaterial {
            wgsl: v.generated.wgsl,
        }),
        Err(errors) => Err(MaterialError::SaveBlocked {
            reason: format!("{} compile error(s)", errors.len()),
            errors,
        }),
    }
}

fn migrate_graph(graph: &mut MaterialGraph, from: u32) -> Option<u32> {
    if from >= FORMAT_VERSION {
        return None;
    }
    for version in from..FORMAT_VERSION {
        migrate_step(graph, version);
    }
    graph.version = FORMAT_VERSION;
    Some(from)
}

fn migrate_step(_graph: &mut MaterialGraph, from: u32) {
    match from {
        // v0 -> v1: only the version field was missing in hand-written files.
        0 => {}
        other => panic!("no migration from matgraph format v{other}"),
    }
}

fn io_error(path: &Path, source: std::io::Error) -> MaterialError {
    MaterialError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn backup_path(path: &Path, from_version: u32) -> PathBuf {
    let mut name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("material.matgraph")
        .to_string();
    name.push_str(&format!(".v{from_version}.bak"));
    path.with_file_name(name)
}
