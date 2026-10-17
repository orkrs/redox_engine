//! Shape casts, overlap queries and collision layers.

use rapier3d::parry::query::ShapeCastOptions;
use rapier3d::prelude::{
    Collider, ColliderHandle, Group, InteractionGroups, InteractionTestMode, Pose, QueryFilter,
    SharedShape,
};
use redox_ecs::Entity;
use redox_math::{Quat, Vec3};

use crate::context::{PhysicsContext, PhysicsError};

/// Directions shorter than this are treated as "no direction".
const MIN_DIRECTION_LENGTH: f32 = 1e-6;

/// A convex shape used for sweeps and overlap tests.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QueryShape {
    /// A ball of the given radius.
    Sphere {
        /// Radius in metres.
        radius: f32,
    },
    /// A capsule aligned with the local Y axis.
    Capsule {
        /// Half the length of the cylindrical part, in metres.
        half_height: f32,
        /// Radius of the hemispherical caps, in metres.
        radius: f32,
    },
    /// An axis-aligned (in local space) box.
    Cuboid {
        /// Half the size along each local axis, in metres.
        half_extents: Vec3,
    },
}

impl QueryShape {
    /// Builds the rapier shape used by the physics queries.
    fn to_shared(self) -> SharedShape {
        match self {
            Self::Sphere { radius } => SharedShape::ball(radius),
            Self::Capsule {
                half_height,
                radius,
            } => SharedShape::capsule_y(half_height, radius),
            Self::Cuboid { half_extents } => {
                SharedShape::cuboid(half_extents.x, half_extents.y, half_extents.z)
            }
        }
    }
}

/// The result of a successful shape cast.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeHit {
    /// Entity owning the hit collider, if it is mapped to one.
    pub entity: Option<Entity>,
    /// Distance travelled by the shape before the impact.
    pub distance: f32,
    /// World-space contact point on the hit surface.
    pub point: Vec3,
    /// Surface normal of the hit, facing the caster.
    pub normal: Vec3,
}

/// Collision group membership and filter, as bit masks.
///
/// Two colliders interact only when each one's `member_of` shares a bit with the other's
/// `collides_with`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollisionLayers {
    /// Layers this collider belongs to.
    pub member_of: u32,
    /// Layers this collider is willing to interact with.
    pub collides_with: u32,
}

impl Default for CollisionLayers {
    fn default() -> Self {
        Self::ALL
    }
}

impl CollisionLayers {
    /// Member of every layer and collides with every layer.
    pub const ALL: Self = Self {
        member_of: u32::MAX,
        collides_with: u32::MAX,
    };

    /// Member of no layer and collides with nothing.
    pub const NONE: Self = Self {
        member_of: 0,
        collides_with: 0,
    };

    /// Creates layers from explicit membership and filter masks.
    pub const fn new(member_of: u32, collides_with: u32) -> Self {
        Self {
            member_of,
            collides_with,
        }
    }

    /// Converts the layers into rapier's collision groups.
    pub fn to_interaction_groups(self) -> InteractionGroups {
        InteractionGroups::new(
            Group::from_bits_retain(self.member_of),
            Group::from_bits_retain(self.collides_with),
            InteractionTestMode::And,
        )
    }
}

/// Returns true when the collider's memberships share a bit with `layers`.
fn layer_matches(collider: &Collider, layers: u32) -> bool {
    collider.collision_groups().memberships.bits() & layers != 0
}

/// Converts a direction into a unit vector, or `None` when it is (nearly) zero.
fn unit_direction(direction: Vec3) -> Option<Vec3> {
    let length = direction.length();
    (length >= MIN_DIRECTION_LENGTH).then(|| direction / length)
}

impl PhysicsContext {
    /// Sweeps `shape` from `position` along `direction` for at most `max_distance`.
    ///
    /// `direction` is normalised; a (near) zero direction returns `None`. Colliders of the
    /// `exclude` entities and colliders whose layers do not match `layers` are ignored.
    /// Returns the first surface hit, if any.
    pub fn cast_shape(
        &self,
        shape: QueryShape,
        position: Vec3,
        rotation: Quat,
        direction: Vec3,
        max_distance: f32,
        exclude: &[Entity],
        layers: u32,
    ) -> Option<ShapeHit> {
        let direction = unit_direction(direction)?;
        let excluded = self.excluded_colliders(exclude);
        let predicate = |handle: ColliderHandle, collider: &Collider| {
            !excluded.contains(&handle) && layer_matches(collider, layers)
        };
        let filter = QueryFilter::default().predicate(&predicate);
        let shared = shape.to_shared();
        let options = ShapeCastOptions::with_max_time_of_impact(max_distance);

        let (handle, hit) = self.query_pipeline(filter).cast_shape(
            &Pose::from_parts(position, rotation),
            direction,
            &*shared,
            options,
        )?;

        // The hit collider is the composite side of the cast: its witness and normal (1) are
        // in world space, while (2) belongs to the caster's local frame.
        let normal = if hit.normal1.dot(direction) > 0.0 {
            -hit.normal1
        } else {
            hit.normal1
        };
        Some(ShapeHit {
            entity: self.entity_of_collider(handle),
            distance: hit.time_of_impact,
            point: hit.witness1,
            normal,
        })
    }

    /// Returns every entity with a collider overlapping `shape` placed at `position`.
    ///
    /// Each entity appears once even when it owns several colliders. Filtering works as in
    /// [`PhysicsContext::cast_shape`].
    pub fn overlap_shape(
        &self,
        shape: QueryShape,
        position: Vec3,
        rotation: Quat,
        exclude: &[Entity],
        layers: u32,
    ) -> Vec<Entity> {
        self.overlapping_entities(shape, position, rotation, exclude, layers, false)
    }

    /// Returns true when at least one collider overlaps `shape` placed at `position`.
    pub fn overlap_any(
        &self,
        shape: QueryShape,
        position: Vec3,
        rotation: Quat,
        exclude: &[Entity],
        layers: u32,
    ) -> bool {
        !self
            .overlapping_entities(shape, position, rotation, exclude, layers, true)
            .is_empty()
    }

    /// Sets the collision groups of every collider attached to `entity`.
    ///
    /// Returns [`PhysicsError::EntityNotFound`] when the entity has no body.
    pub fn set_collider_layers(
        &mut self,
        entity: Entity,
        layers: CollisionLayers,
    ) -> Result<(), PhysicsError> {
        let handles = self
            .collider_handles(entity)
            .ok_or(PhysicsError::EntityNotFound)?
            .to_vec();
        let groups = layers.to_interaction_groups();
        for handle in handles {
            if let Some(collider) = self.colliders.get_mut(handle) {
                collider.set_collision_groups(groups);
            }
        }
        Ok(())
    }

    /// Shared overlap implementation; stops at the first hit when `first_only` is set.
    fn overlapping_entities(
        &self,
        shape: QueryShape,
        position: Vec3,
        rotation: Quat,
        exclude: &[Entity],
        layers: u32,
        first_only: bool,
    ) -> Vec<Entity> {
        let excluded = self.excluded_colliders(exclude);
        let predicate = |handle: ColliderHandle, collider: &Collider| {
            !excluded.contains(&handle) && layer_matches(collider, layers)
        };
        let filter = QueryFilter::default().predicate(&predicate);
        let shared = shape.to_shared();
        let pipeline = self.query_pipeline(filter);

        let mut entities: Vec<Entity> = Vec::new();
        for (handle, _) in pipeline.intersect_shape(Pose::from_parts(position, rotation), &*shared)
        {
            let Some(entity) = self.entity_of_collider(handle) else {
                continue;
            };
            if !entities.contains(&entity) {
                entities.push(entity);
            }
            if first_only {
                break;
            }
        }
        entities
    }
}
