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
/// Binds runtime handles from stored asset references.
pub mod asset_resolve;
/// Registration of the scene component types for reflection.
pub mod register;
/// The level open in the editor: a scene file or a world folder.
pub mod level;
/// The open world: partition streamer, manifest and root directory.
pub mod world_session;

pub use asset_resolve::bind_render_handles;
pub use document::{
    LoadReport, LoadWarning, ResolvedAssetRef, SceneDocument, SceneError, refresh_ref_paths,
};
pub use format::SCENE_FORMAT_VERSION;
pub use git::{ActorGitStatus, WorldGitTracker};
pub use register::register_scene_components;
pub use level::{Level, LevelError, LevelKind};
pub use world_session::WorldSession;
