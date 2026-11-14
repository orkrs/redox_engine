//! Levels and world sessions of RedOx Studio: the open streaming world and its version-control status.

#![warn(missing_docs)]

/// Version-control status of actor files.
pub mod git;
/// Two-way index between scene entities and their stable guids.
pub mod guid_index;
/// On-disk layout of `*.scene.ron` files: format version, entities and migrations.
pub mod format;
/// The scene document: opening and saving `*.scene.ron` files.
pub mod document;
/// Registration of the scene component types for reflection.
pub mod register;
/// The level open in the editor: a scene file or a world folder.
pub mod level;
/// The open world: partition streamer, manifest and root directory.
pub mod world_session;
/// Scene hierarchy for the outliner (no UI).
pub mod outliner_model;

pub use document::{LoadReport, LoadWarning, SceneDocument, SceneError, refresh_ref_paths};
pub use format::SCENE_FORMAT_VERSION;
pub use git::{ActorGitStatus, WorldGitTracker};
pub use register::register_scene_components;
pub use level::{Level, LevelError, LevelKind};
pub use world_session::WorldSession;
pub use outliner_model::{
    FlatRow, OutlinerFilter, OutlinerModel, OutlinerNode, OutlinerWarning, OUTLINER_ROW_HEIGHT,
    VisibleRows, build, flatten, invert_selection, select_children, select_range,
    toggle_select, visible_row_range, SelectionModifiers,
};
