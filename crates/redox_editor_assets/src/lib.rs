//! Asset services of RedOx Studio: GUID registry integration, the reference index of scene files,
//! moves and deletes that keep references intact, the background import queue, and the
//! diagnostics of opening an assets root. Egui-free: the browser and the panels live in
//! `redox_editor_ui`.

#![warn(missing_docs)]

pub mod audit;
pub mod browser_model;
pub mod import;
pub mod import_dialog;
pub mod import_log;
pub mod import_queue;
pub mod import_settings;
pub mod mat_graph_view;
pub mod material_compile;
pub mod material_document;
pub mod material_preview;
pub mod reference_index;
pub mod service;
pub mod thumbnail_service;

pub use browser_model::{
    AssetAction, AssetKind, BrowserEntry, BrowserFilter, BrowserModel, StatusFilter, actions_for,
};
pub use mat_graph_view::{ConnectError, MatGraphUndo, MatGraphView, NodeLayout};
pub use material_compile::{CompileMessage, CompileReport};
pub use material_document::{CompiledMaterial, MaterialDocument, MaterialError, compile_for_save};
pub use material_preview::{
    MaterialPreviewSettings, PreviewLighting, PreviewMesh, SimplePreviewLight,
};

pub use audit::{
    AuditConfig, AuditReport, DuplicateGroup, IncomingReference, OutgoingReference, ReferenceView,
    ReferenceViewerModel, SizeMapCsvRow, SizeMapNode, build_reference_view, build_size_map,
    export_audit_json, export_size_map_csv, export_size_map_json, find_duplicates, find_orphans,
    parse_size_map_csv, run_audit,
};
pub use import::{
    AssetImporter, CancelToken, GltfValidateImporter, ImportError, ImportJob, ImportOutput,
    ImportReport, ImporterRegistry, TextureHeader, TextureProbeImporter, probe_texture_header,
};
pub use import_dialog::{
    ImportDialogJob, ImportDialogModel, ImportDialogPhase, ImportDialogRow, ImportPreview,
};
pub use import_log::{ImportLogEntry, ImportLogFilter, ImportLogModel, ImportLogStatus};
pub use import_queue::{ImportEvent, ImportQueue, MAX_IMPORT_WORKERS, default_worker_count};
pub use import_settings::{
    ImportPresetFile, ImportPresetId, ImportPresetLibrary, MaterialMode, PerAssetImportSettings,
    PresetEntry, SETTINGS_SCHEMA, default_for_kind, parse_settings_blob, read_per_asset_settings,
    settings_blob, settings_fingerprint, settings_for_preset, write_per_asset_settings,
};
pub use reference_index::{
    MissingRef, RefEdit, RefResolution, ReferenceError, ReferenceIndex, SceneRef, is_scene_path,
    resolve_ref,
};
pub use service::{
    AssetChange, AssetMoveOp, AssetReport, AssetService, AssetServiceError, DeleteOutcome,
    ImportCounts, ImportSettings, MoveReport, RefFix,
};
pub use thumbnail_service::{ThumbnailJob, ThumbnailOutcome, ThumbnailService};
