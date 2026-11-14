//! Levels and world sessions of RedOx Studio: the open streaming world and its version-control status.

#![warn(missing_docs)]

/// Binds runtime handles from stored asset references.
pub mod asset_resolve;
/// The scene document: opening and saving `*.scene.ron` files.
pub mod document;
/// On-disk layout of `*.scene.ron` files: format version, entities and migrations.
pub mod format;
/// Version-control status of actor files.
pub mod git;
/// Two-way index between scene entities and their stable guids.
pub mod guid_index;
/// The level open in the editor: a scene file or a world folder.
pub mod level;
/// Scene hierarchy for the outliner (no UI).
pub mod outliner_model;
/// Undoable outliner operations.
pub mod outliner_ops;
/// Registration of the scene component types for reflection.
pub mod register;
/// GUID-level diff between scene file snapshots.
pub mod scene_diff;
/// Git status, commit and pull for scene documents.
pub mod scene_git;
/// Scene validation rules, fixes and project-wide validation (EDT-B7).
pub mod validate;
/// The open world: partition streamer, manifest and root directory.
pub mod world_session;

pub use asset_resolve::bind_render_handles;
pub use document::{
    LoadReport, LoadWarning, ResolvedAssetRef, SceneDocument, SceneError, refresh_ref_paths,
};
pub use format::{SCENE_FORMAT_VERSION, SceneFile};
pub use git::{ActorGitStatus, WorldGitTracker};
pub use level::{Level, LevelError, LevelKind};
pub use outliner_model::{
    FlatRow, OUTLINER_ROW_HEIGHT, OutlinerFilter, OutlinerModel, OutlinerNode, OutlinerWarning,
    SelectionModifiers, VisibleRows, build, flatten, invert_selection, select_children,
    select_range, toggle_select, visible_row_range,
};
pub use outliner_ops::{
    apply_lock, apply_visibility, is_hidden, is_locked, is_pickable, isolate, reparent,
};
pub use register::register_scene_components;
pub use scene_diff::{SceneEntityChange, diff_scene_files};
pub use scene_git::{SceneGitError, SceneGitStatus, load_scene_file};
pub use validate::{
    IssueUserAction, Severity, ValidateLevelOptions, ValidateProjectError, ValidationFix,
    ValidationIssue, ValidationReport, ValidationRule, ValidationSettings, apply_fix,
    apply_issue_action, validate_level, validate_project,
};
pub use world_session::WorldSession;
