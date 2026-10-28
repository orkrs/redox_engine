use rapier3d::dynamics::RigidBodyHandle;
use rapier3d::geometry::ColliderHandle;
use redox_math::Vec3;

/// Component indicating that an entity has a rigid body in the physics world.
#[derive(Debug, Clone)]
pub struct RigidBody {
    /// Handle of the rapier rigid body.
    pub handle: RigidBodyHandle,
}

/// Component indicating that an entity has a collider.
#[derive(Debug, Clone)]
pub struct Collider {
    /// Handle of the rapier collider.
    pub handle: ColliderHandle,
}

/// Component to control or read the linear and angular velocity of a rigid body.
#[derive(Debug, Clone, Copy, Default)]
pub struct Velocity {
    /// Linear velocity in world units per second.
    pub linvel: Vec3,
    /// Angular velocity in radians per second.
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
    /// Downward acceleration applied each step, in units per second squared.
    pub gravity: f32,
    /// Whether the character stood on ground after the last move.
    pub grounded: bool,
    /// Vertical speed in units per second, positive is up. Owned by the controller.
    pub vertical_velocity: f32,
    /// Speed of a jump set by `jump`, units per second.
    pub jump_speed: f32,
}

impl CharacterController {
    /// Starts a jump if the character stood on the ground after the last move. Returns whether it started.
    pub fn jump(&mut self) -> bool {
        if !self.grounded {
            return false;
        }
        self.vertical_velocity = self.jump_speed;
        self.grounded = false;
        true
    }
}

impl Default for CharacterController {
    fn default() -> Self {
        Self {
            desired_translation: Vec3::ZERO,
            gravity: 20.0,
            grounded: false,
            vertical_velocity: 0.0,
            jump_speed: 7.0,
        }
    }
}

/// Sensor collider that emits [`crate::trigger::TriggerEnter`] / `TriggerExit`.
#[derive(Debug, Clone, Default)]
pub struct Trigger;
