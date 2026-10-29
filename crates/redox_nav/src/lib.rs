//! Navigation subsystem for the RedOx Engine: navmesh baking, pathfinding and avoidance.
//!
//! The public API is owned by this crate; the `rerecast` and `landmass` backends
//! never appear in its signatures.
//!
//! Baking turns level geometry into tiles ([`bake`], [`from_triangle_meshes`]); the tiles are
//! stored in the `.nav` format ([`NavData::to_bytes`] / [`NavData::from_bytes`]). A
//! [`NavWorld`] built from them answers path and point queries. Agents are ECS entities
//! ([`NavAgent`], [`NavTarget`]) moved by [`nav_path_system`] and [`nav_follow_system`].

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod avoid;
mod backend;
mod bake;
mod blockers;
mod components;
mod data;
mod debug;
mod error;
mod format;
mod geometry;
mod links;
mod query;
mod queue;
mod systems;
mod voxel;
mod world;

pub use avoid::{AgentStatus, AvoidanceParams, NavAgentId};
pub use bake::{NavGeometry, bake};
pub use blockers::NavBlockerId;
pub use components::{
    NavAgent, NavDesiredVelocity, NavHandle, NavPath, NavTarget, NavVelocity, PathFailed,
    PathReached,
};
pub use data::{AgentParams, NavAreaId, NavData, NavPolygon, NavTile};
pub use debug::NavDebugDraw;
pub use error::{BakeError, NavError, NavFormatError};
pub use geometry::from_triangle_meshes;
pub use links::{LINK_MAX_OFF_MESH, NavLink, NavLinkId};
pub use queue::{PathQueue, PathTicket};
pub use systems::{
    ARRIVE_DISTANCE, PATH_QUERIES_PER_FRAME, nav_avoid_system, nav_follow_system, nav_move_system,
    nav_path_system,
};
pub use voxel::{VoxelConfig, voxel_config};
pub use world::{AgentClass, NavWorld};
