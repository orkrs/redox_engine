//! Asset services of RedOx Studio: GUID registry integration, the reference index of scene files,
//! moves and deletes that keep references intact, the background import queue, and the
//! diagnostics of opening an assets root. Egui-free: the browser and the panels live in
//! `redox_editor_ui`.

#![warn(missing_docs)]

pub mod browser_model;
pub mod import;
pub mod mat_graph_view;
pub mod material_compile;
pub mod material_document;
pub mod material_preview;
pub mod import_queue;
pub mod reference_index;
pub mod service;

pub use browser_model::{
    AssetAction, AssetKind, BrowserEntry, BrowserFilter, BrowserModel, StatusFilter, actions_for,
};
pub use mat_graph_view::{ConnectError, MatGraphUndo, MatGraphView, NodeLayout};
pub use material_compile::{CompileMessage, CompileReport};
pub use material_document::{
    CompiledMaterial, MaterialDocument, MaterialError, compile_for_save,
};
pub use material_preview::{
    MaterialPreviewSettings, PreviewLighting, PreviewMesh, SimplePreviewLight,
};

pub use import::{
    AssetImporter, CancelToken, GltfValidateImporter, ImportError, ImportJob, ImportOutput,
    ImportReport, ImporterRegistry, TextureHeader, TextureProbeImporter, probe_texture_header,
};
pub use import_queue::{ImportEvent, ImportQueue, MAX_IMPORT_WORKERS, default_worker_count};
pub use reference_index::{
    MissingRef, RefEdit, RefResolution, ReferenceError, ReferenceIndex, SceneRef, is_scene_path,
    resolve_ref,
};
pub use service::{
    AssetChange, AssetMoveOp, AssetReport, AssetService, AssetServiceError, DeleteOutcome,
    ImportCounts, ImportSettings, MoveReport, RefFix,
};
