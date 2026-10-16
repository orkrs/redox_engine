use rapier3d::dynamics::RigidBodyHandle;
use rapier3d::geometry::ColliderHandle;
use redox_math::Vec3;

/// Component indicating that an entity has a rigid body in the physics world.
#[derive(Debug, Clone)]
pub struct RigidBody {
    pub handle: RigidBodyHandle,
}

/// Component indicating that an entity has a collider.
#[derive(Debug, Clone)]
pub struct Collider {
    pub handle: ColliderHandle,
}

/// Component to control or read the linear and angular velocity of a rigid body.
#[derive(Debug, Clone, Copy, Default)]
pub struct Velocity {
    pub linvel: Vec3,
    pub angvel: Vec3,
}

/// Stores the transform before the physics step for render interpolation (alpha blending).
#[derive(Debug, Clone, Copy, Default)]
pub struct PreviousTransform(pub redox_math::Transform);

/// Marker component for kinematic bodies (driven by `Transform`, not physics).
#[derive(Debug, Clone, Default)]
pub struct Kinematic;

/// Rapier kinematic character: wall slide, autostep, slope limit, gravity.
#[derive(Debug, Clone)]
pub struct CharacterController {
    /// Horizontal (and optional jump) wish this frame, **not** including gravity.
    pub desired_translation: Vec3,
    pub gravity: f32,
    pub grounded: bool,
}

impl Default for CharacterController {
    fn default() -> Self {
        Self {
            desired_translation: Vec3::ZERO,
            gravity: 20.0,
            grounded: false,
        }
    }
}

/// Sensor collider that emits [`crate::trigger::TriggerEnter`] / `TriggerExit`.
#[derive(Debug, Clone, Default)]
pub struct Trigger;
