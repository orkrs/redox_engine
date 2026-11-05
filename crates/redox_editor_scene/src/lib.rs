//! Levels and world sessions of RedOx Studio: the open streaming world and its version-control status.

#![warn(missing_docs)]

/// Version-control status of actor files.
pub mod git;
/// The level open in the editor: a scene file or a world folder.
pub mod level;
/// The open world: partition streamer, manifest and root directory.
pub mod world_session;

pub use git::{ActorGitStatus, WorldGitTracker};
pub use level::{Level, LevelError, LevelKind};
pub use world_session::WorldSession;
