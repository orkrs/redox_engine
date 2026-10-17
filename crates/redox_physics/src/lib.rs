//! Physics subsystem for the RedOx Engine based on rapier3d.

#![warn(missing_docs)]

pub mod character;
/// ECS components of the physics integration.
pub mod components;
/// Contact enter/exit events between solid colliders and physics materials.
pub mod contacts;
/// The `PhysicsContext` resource owning the rapier world.
pub mod context;
/// Joints between rigid bodies.
pub mod joints;
/// Convenience re-exports of the physics modules.
pub mod physics;
/// Raycast requests and results.
pub mod raycast;
/// Shape casts, overlap queries and collision layers.
pub mod shapes;
/// ECS to rapier synchronisation and the fixed-step tick.
pub mod sync;
pub mod trigger;

pub use character::move_character_controllers;
pub use components::{CharacterController, Collider, Kinematic, RigidBody, Trigger, Velocity};
pub use contacts::{ContactEnter, ContactEvent, ContactExit, PhysicsMaterial};
pub use context::{PhysicsContext, PhysicsError};
pub use joints::{JointBroken, JointDesc, JointId, JointKind};
pub use raycast::{RaycastRequest, RaycastResult};
pub use shapes::{ClosestPoint, CollisionLayers, QueryShape, ShapeHit};
pub use sync::{step_physics, sync_from_physics, sync_to_physics, tick_physics};
pub use trigger::{TriggerEnter, TriggerExit};
