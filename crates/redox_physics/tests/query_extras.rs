//! Specification (tests first) for the remaining shape queries (plan item B2).
//!
//! API to implement in `redox_physics` (in `shapes.rs`; `ShapeHit` and `QueryShape` already exist):
//!
//! ```text
//! impl PhysicsContext {
//!     /// Sweeps `shape` and returns the first hit on EVERY entity along the way, nearest first
//!     /// (one `ShapeHit` per entity). Same filtering as `cast_shape`.
//!     pub fn cast_shape_all(&self, shape: QueryShape, position: Vec3, rotation: Quat, direction: Vec3,
//!                           max_distance: f32, exclude: &[Entity], layers: u32) -> Vec<ShapeHit>;
//!     /// Entities with a collider that contains `point` (each entity once).
//!     pub fn overlap_point(&self, point: Vec3, exclude: &[Entity], layers: u32) -> Vec<Entity>;
//!     /// The nearest point on any collider surface within `max_distance` of `point`.
//!     /// A point inside a collider returns that point itself with distance 0.
//!     pub fn closest_point(&self, point: Vec3, max_distance: f32, exclude: &[Entity], layers: u32)
//!         -> Option<ClosestPoint>;
//! }
//! #[derive(Debug, Clone, Copy, PartialEq)]
//! pub struct ClosestPoint { pub entity: Option<Entity>, pub point: Vec3, pub distance: f32 }
//! ```
//!
//! Export `ClosestPoint` from the crate root. `direction` of zero length gives an empty list.

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::{Entity, World};
use redox_math::{Quat, Vec3};
use redox_physics::{CollisionLayers, PhysicsContext, QueryShape};

const ALL: u32 = u32::MAX;
const SPHERE: QueryShape = QueryShape::Sphere { radius: 0.5 };

/// Three walls 1 m thick centred at z = -5, -10 and -15 (front faces at -4.5, -9.5, -14.5).
fn walls() -> (PhysicsContext, [Entity; 3]) {
    let mut world = World::new();
    let entities = [world.spawn(), world.spawn(), world.spawn()];
    let mut ctx = PhysicsContext::new(Vec3::ZERO);
    for (i, entity) in entities.iter().enumerate() {
        ctx.add_rigid_body(
            *entity,
            RigidBodyBuilder::fixed().translation(Vec3::new(0.0, 0.0, -5.0 - 5.0 * i as f32)),
            vec![ColliderBuilder::cuboid(5.0, 5.0, 0.5)],
        )
        .unwrap();
    }
    ctx.step(1.0 / 60.0);
    (ctx, entities)
}

fn sweep(ctx: &PhysicsContext, max: f32, layers: u32) -> Vec<redox_physics::ShapeHit> {
    ctx.cast_shape_all(
        SPHERE,
        Vec3::ZERO,
        Quat::IDENTITY,
        Vec3::NEG_Z,
        max,
        &[],
        layers,
    )
}

#[test]
fn cast_all_returns_every_entity_nearest_first() {
    let (ctx, walls) = walls();
    let hits = sweep(&ctx, 100.0, ALL);
    assert_eq!(hits.len(), 3, "{hits:?}");
    let order: Vec<_> = hits.iter().map(|h| h.entity).collect();
    assert_eq!(order, vec![Some(walls[0]), Some(walls[1]), Some(walls[2])]);
    // Sphere radius 0.5: it touches a front face (z = -4.5) after travelling 4.0.
    for (hit, expected) in hits.iter().zip([4.0, 9.0, 14.0]) {
        assert!(
            (hit.distance - expected).abs() < 0.05,
            "distance {} vs {expected}",
            hit.distance
        );
    }
    assert!(hits.windows(2).all(|w| w[0].distance <= w[1].distance));
}

#[test]
fn cast_all_respects_the_maximum_distance() {
    let (ctx, walls) = walls();
    let hits = sweep(&ctx, 12.0, ALL);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[1].entity, Some(walls[1]));
    assert!(sweep(&ctx, 1.0, ALL).is_empty());
}

#[test]
fn cast_all_respects_excludes_and_layers() {
    let (mut ctx, walls) = walls();
    let hits = ctx.cast_shape_all(
        SPHERE,
        Vec3::ZERO,
        Quat::IDENTITY,
        Vec3::NEG_Z,
        100.0,
        &[walls[0]],
        ALL,
    );
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].entity, Some(walls[1]));

    ctx.set_collider_layers(walls[1], CollisionLayers::new(0b10, ALL))
        .unwrap();
    let only_default = sweep(&ctx, 100.0, 0b01);
    // walls[0] and walls[2] still belong to every layer; walls[1] only to layer 2.
    let entities: Vec<_> = only_default.iter().map(|h| h.entity).collect();
    assert_eq!(entities, vec![Some(walls[0]), Some(walls[2])]);
}

#[test]
fn cast_all_with_no_direction_is_empty() {
    let (ctx, _) = walls();
    let hits = ctx.cast_shape_all(
        SPHERE,
        Vec3::ZERO,
        Quat::IDENTITY,
        Vec3::ZERO,
        100.0,
        &[],
        ALL,
    );
    assert!(hits.is_empty());
}

#[test]
fn overlap_point_finds_the_containing_entities() {
    let (ctx, walls) = walls();
    let inside = ctx.overlap_point(Vec3::new(1.0, 1.0, -10.0), &[], ALL);
    assert_eq!(inside, vec![walls[1]]);
    assert!(
        ctx.overlap_point(Vec3::new(1.0, 1.0, -7.5), &[], ALL)
            .is_empty(),
        "the gap between walls is empty"
    );
    assert!(
        ctx.overlap_point(Vec3::new(1.0, 1.0, -10.0), &[walls[1]], ALL)
            .is_empty(),
        "excluded"
    );
    assert!(
        ctx.overlap_point(Vec3::new(1.0, 1.0, -10.0), &[], 0)
            .is_empty(),
        "layer mask 0 matches nothing"
    );
}

#[test]
fn closest_point_projects_onto_the_nearest_surface() {
    let (ctx, walls) = walls();
    // 3 m in front of the first wall's front face (z = -4.5): the origin is 4.5 away from it.
    let hit = ctx
        .closest_point(Vec3::new(2.0, 1.0, -1.5), 10.0, &[], ALL)
        .unwrap();
    assert_eq!(hit.entity, Some(walls[0]));
    assert!((hit.point - Vec3::new(2.0, 1.0, -4.5)).length() < 1e-3);
    assert!((hit.distance - 3.0).abs() < 1e-3);

    assert!(
        ctx.closest_point(Vec3::new(2.0, 1.0, -1.5), 2.0, &[], ALL)
            .is_none(),
        "farther than max_distance"
    );
    let skipped = ctx
        .closest_point(Vec3::new(2.0, 1.0, -1.5), 10.0, &[walls[0]], ALL)
        .unwrap();
    assert_eq!(skipped.entity, Some(walls[1]));
}

#[test]
fn closest_point_inside_a_collider_is_the_point_itself() {
    let (ctx, walls) = walls();
    let p = Vec3::new(1.0, 2.0, -10.0);
    let hit = ctx.closest_point(p, 10.0, &[], ALL).unwrap();
    assert_eq!(hit.entity, Some(walls[1]));
    assert_eq!(hit.distance, 0.0);
    assert!((hit.point - p).length() < 1e-4);
}
