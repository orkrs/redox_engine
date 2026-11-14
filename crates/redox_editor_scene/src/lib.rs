//! Levels and world sessions of RedOx Studio: the open streaming world and its version-control status.

#![warn(missing_docs)]

/// Version-control status of actor files.
pub mod git;
/// GUID-level diff between scene file snapshots.
pub mod scene_diff;
/// Git status, commit and pull for scene documents.
pub mod scene_git;
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

pub use document::{LoadReport, LoadWarning, SceneDocument, SceneError, refresh_ref_paths};
pub use format::SCENE_FORMAT_VERSION;
pub use git::{ActorGitStatus, WorldGitTracker};
pub use scene_diff::{SceneEntityChange, diff_scene_files};
pub use scene_git::{SceneGitError, SceneGitStatus, load_scene_file};
pub use register::register_scene_components;
pub use level::{Level, LevelError, LevelKind};
pub use world_session::WorldSession;
