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
#[derive(Debug, Clone, Default)]
pub struct Velocity {
    pub linvel: Vec3,
    pub angvel: Vec3,
}

/// Marker component for kinematic bodies (driven by `Transform`, not physics).
#[derive(Debug, Clone, Default)]
pub struct Kinematic;
