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
/// The open world: partition streamer, manifest and root directory.
pub mod world_session;

pub use document::{LoadReport, LoadWarning, SceneDocument, SceneError};
pub use format::SCENE_FORMAT_VERSION;
pub use git::{ActorGitStatus, WorldGitTracker};
pub use register::register_scene_components;
pub use world_session::WorldSession;
