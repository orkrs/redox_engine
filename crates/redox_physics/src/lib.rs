//! Physics subsystem for the RedOx Engine based on rapier3d.

#![warn(missing_docs)]

pub mod character;
/// ECS components of the physics integration.
pub mod components;
/// The `PhysicsContext` resource owning the rapier world.
pub mod context;
/// Convenience re-exports of the physics modules.
pub mod physics;
/// Raycast requests and results.
pub mod raycast;
/// ECS to rapier synchronisation and the fixed-step tick.
pub mod sync;
pub mod trigger;

pub use character::move_character_controllers;
pub use components::{CharacterController, Collider, Kinematic, RigidBody, Trigger, Velocity};
pub use context::{PhysicsContext, PhysicsError};
pub use raycast::{RaycastRequest, RaycastResult};
pub use sync::{step_physics, sync_from_physics, sync_to_physics, tick_physics};
pub use trigger::{TriggerEnter, TriggerExit};
