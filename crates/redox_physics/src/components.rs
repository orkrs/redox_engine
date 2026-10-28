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

/// Movement tuning of a [`CharacterController`]; lengths in world units, angles in radians.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CharacterSettings {
    /// Highest obstacle climbed without jumping; 0.0 disables autostep.
    pub step_height: f32,
    /// Free room in front of the character needed to start a step (rapier `min_width`).
    pub step_min_width: f32,
    /// Steepest slope that can be walked up.
    pub max_slope_angle: f32,
    /// Distance below the feet at which the character is pulled back to the ground; 0.0 disables.
    pub snap_to_ground: f32,
}

impl Default for CharacterSettings {
    fn default() -> Self {
        Self {
            step_height: 0.3,
            step_min_width: 0.1,
            max_slope_angle: std::f32::consts::FRAC_PI_4,
            snap_to_ground: 0.2,
        }
    }
}

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
    /// Movement tuning: steps, slopes and snapping to the ground.
    pub settings: CharacterSettings,
    /// Collider the character stood on after the last move (the one `grounded` refers to). Controller state.
    pub ground_collider: Option<ColliderHandle>,
    /// Ground collider and its position at the last step, used to carry the character with a moving
    /// platform. Controller state.
    pub ground_pose: Option<(ColliderHandle, Vec3)>,
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
            settings: CharacterSettings::default(),
            ground_collider: None,
            ground_pose: None,
        }
    }
}

/// Sensor collider that emits [`crate::trigger::TriggerEnter`] / `TriggerExit`.
#[derive(Debug, Clone, Default)]
pub struct Trigger;
