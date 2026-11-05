//! Levels and world sessions of RedOx Studio: the open streaming world and its version-control status.

#![warn(missing_docs)]

/// Version-control status of actor files.
pub mod git;
/// Registration of the scene component types for reflection.
pub mod register;
/// The open world: partition streamer, manifest and root directory.
pub mod world_session;

pub use git::{ActorGitStatus, WorldGitTracker};
pub use register::register_scene_components;
pub use world_session::WorldSession;
