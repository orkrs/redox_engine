//! Physics subsystem for the RedOx Engine based on rapier3d.

pub mod character;
pub mod components;
pub mod context;
pub mod physics;
pub mod raycast;
pub mod sync;
pub mod trigger;
pub mod utils;

pub use character::move_character_controllers;
pub use components::{CharacterController, Collider, Kinematic, RigidBody, Trigger, Velocity};
pub use context::{PhysicsContext, PhysicsError};
pub use raycast::{RaycastRequest, RaycastResult};
pub use sync::{step_physics, sync_from_physics, sync_to_physics, tick_physics};
pub use trigger::{TriggerEnter, TriggerExit};
