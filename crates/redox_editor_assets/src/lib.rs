//! Asset services of RedOx Studio: GUID registry integration, the reference index of scene files,
//! moves and deletes that keep references intact, and the diagnostics of opening an assets root.
//! Egui-free: the browser and the diagnostics panel live in `redox_editor_ui`.

#![warn(missing_docs)]

pub mod reference_index;
pub mod service;

pub use reference_index::{
    MissingRef, RefEdit, RefResolution, ReferenceError, ReferenceIndex, SceneRef, is_scene_path,
    resolve_ref,
};
pub use service::{
    AssetChange, AssetMoveOp, AssetReport, AssetService, AssetServiceError, DeleteOutcome,
    MoveReport, RefFix,
};
