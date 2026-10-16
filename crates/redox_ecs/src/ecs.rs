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
//! // Mutable fetches require exclusive world access
//! let q = Query::<(&Position, &mut Velocity), With<Player>>::new();
//! for (pos, vel) in q.iter_mut(&mut world) { ... }
//! ```

#![warn(clippy::undocumented_unsafe_blocks)]

pub mod archetype;
pub mod component;
pub mod entity;
pub mod event;
pub mod hierarchy;
pub mod query;
pub mod world;

pub use component::Component;
pub use entity::{Entity, EntityAllocator};
pub use event::{EventReader, Events};
pub use hierarchy::{Children, Parent, set_parent, transform_hierarchy_system};
pub use query::filter::{Added, Changed};
pub use query::{NullFilter, With, Without};
pub use query::{ParallelQuery, Query, QueryData, QueryFilter, ReadOnlyQueryData};
pub use world::{Bundle, World};

/// The current version of the `redox_ecs` crate.
pub const REDOX_ECS_VERSION: &str = env!("CARGO_PKG_VERSION");
