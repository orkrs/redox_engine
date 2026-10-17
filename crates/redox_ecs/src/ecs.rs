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

#![warn(missing_docs)]
#![warn(clippy::undocumented_unsafe_blocks)]

/// Archetype tables and typed column storage.
pub mod archetype;
pub mod commands;
/// Component trait and runtime component metadata.
pub mod component;
/// Entity handles and their allocator.
pub mod entity;
pub mod event;
/// Parent/child relations and transform propagation.
pub mod hierarchy;
pub mod query;
/// The `World`: entities, components and resources.
pub mod world;

pub use commands::{CommandQueue, Commands};
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
