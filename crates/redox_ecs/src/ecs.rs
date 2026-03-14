//! The core ECS (Entity-Component System) module for RedOx Engine.
//!
//! This crate provides a high-performance archetype-based ECS implementation
//! with support for queries, events, hierarchies, and parallel iteration.
//!
//! # Tuple queries
//!
//! The query system supports fetching multiple components at once:
//!
//! ```rust,ignore
//! // Iterate over all entities that have both Position and Velocity
//! let q = Query::<(&Position, &Velocity)>::new();
//! for (pos, vel) in q.iter(&world) { ... }
//!
//! // Filtered: only entities that also have the Player component
//! let q = Query::<(&Position, &mut Velocity), With<Player>>::new();
//! for (pos, vel) in q.iter(&world) { ... }
//! ```

pub mod entity;
pub mod component;
pub mod archetype;
pub mod world;
pub mod query;
pub mod event;
pub mod system;
pub mod hierarchy;

pub use entity::{Entity, EntityAllocator};
pub use component::Component;
pub use world::World;
pub use query::{Query, ParallelQuery, QueryData, QueryFilter};
pub use query::{With, Without, NullFilter};
pub use query::filter::{Changed, Added};
pub use event::{Events, EventReader};
pub use system::{System, SystemStage};
pub use hierarchy::{Parent, Children};

/// The current version of the `redox_ecs` crate.
pub const REDOX_ECS_VERSION: &str = env!("CARGO_PKG_VERSION");
