//! Specification (tests first) for shape casts, overlap queries and collision layers.
//!
//! API to implement in `redox_physics` (re-exported from the crate root):
//!
//! ```text
//! pub enum QueryShape { Sphere { radius: f32 }, Capsule { half_height: f32, radius: f32 }, Cuboid { half_extents: Vec3 } }
//! pub struct ShapeHit { pub entity: Option<Entity>, pub distance: f32, pub point: Vec3, pub normal: Vec3 }
//! pub struct CollisionLayers { pub member_of: u32, pub collides_with: u32 }   // Default = ALL / ALL
//! impl CollisionLayers { pub const ALL: Self; pub const NONE: Self; pub fn new(member_of: u32, collides_with: u32) -> Self }
//! impl PhysicsContext {
//!     pub fn cast_shape(&self, shape: QueryShape, position: Vec3, rotation: Quat, direction: Vec3,
//!                       max_distance: f32, exclude: &[Entity], layers: u32) -> Option<ShapeHit>;
//!     pub fn overlap_shape(&self, shape: QueryShape, position: Vec3, rotation: Quat,
//!                          exclude: &[Entity], layers: u32) -> Vec<Entity>;
//!     pub fn overlap_any(&self, shape: QueryShape, position: Vec3, rotation: Quat,
//!                        exclude: &[Entity], layers: u32) -> bool;
//!     pub fn set_collider_layers(&mut self, entity: Entity, layers: CollisionLayers) -> Result<(), PhysicsError>;
//! }
//! ```
//!
//! `layers` in the queries is a bit mask: only colliders whose `member_of` shares a bit with it are
//! considered (`u32::MAX` = everything).

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::{Entity, World};
use redox_math::{Quat, Vec3};
use redox_physics::{CollisionLayers, PhysicsContext, QueryShape};

const ALL: u32 = u32::MAX;

/// A wall 1 m thick at z = -10 (front face at z = -9.5) and a ball far to the side.
fn scene() -> (PhysicsContext, Entity, Entity) {
    let mut world = World::new();
    let wall = world.spawn();
    let ball = world.spawn();
    let mut ctx = PhysicsContext::new(Vec3::ZERO);
    ctx.add_rigid_body(
        wall,
        RigidBodyBuilder::fixed().translation(Vec3::new(0.0, 0.0, -10.0)),
        vec![ColliderBuilder::cuboid(5.0, 5.0, 0.5)],
    )
    .unwrap();
    ctx.add_rigid_body(
        ball,
        RigidBodyBuilder::fixed().translation(Vec3::new(20.0, 0.0, 0.0)),
        vec![ColliderBuilder::ball(1.0)],
    )
    .unwrap();
    ctx.step(1.0 / 60.0); // builds the broad phase used by queries
    (ctx, wall, ball)
}

fn cast(ctx: &PhysicsContext, shape: QueryShape, direction: Vec3, max: f32, exclude: &[Entity], layers: u32) -> Option<redox_physics::ShapeHit> {
    ctx.cast_shape(shape, Vec3::ZERO, Quat::IDENTITY, direction, max, exclude, layers)
}

#[test]
fn a_sphere_stops_one_radius_before_the_wall() {
    let (ctx, wall, _) = scene();
    let hit = cast(&ctx, QueryShape::Sphere { radius: 1.0 }, Vec3::NEG_Z, 50.0, &[], ALL).expect("hits the wall");
    assert_eq!(hit.entity, Some(wall));
    assert!((hit.distance - 8.5).abs() < 1e-2, "9.5 to the face minus the radius: {}", hit.distance);
    assert!(hit.normal.z > 0.9, "normal faces the caster: {:?}", hit.normal);
    assert!((hit.point.z + 9.5).abs() < 1e-2, "contact point on the face: {:?}", hit.point);
}

#[test]
fn capsules_and_boxes_are_swept_with_their_own_extent() {
    let (ctx, wall, _) = scene();
    let capsule = QueryShape::Capsule { half_height: 0.5, radius: 0.5 };
    let hit = cast(&ctx, capsule, Vec3::NEG_Z, 50.0, &[], ALL).unwrap();
    assert_eq!(hit.entity, Some(wall));
    assert!((hit.distance - 9.0).abs() < 1e-2, "capsule radius 0.5: {}", hit.distance);

    let cuboid = QueryShape::Cuboid { half_extents: Vec3::new(1.0, 1.0, 2.0) };
    let hit = cast(&ctx, cuboid, Vec3::NEG_Z, 50.0, &[], ALL).unwrap();
    assert!((hit.distance - 7.5).abs() < 1e-2, "box half depth 2: {}", hit.distance);
}

#[test]
fn a_rotated_box_is_swept_in_its_rotated_frame() {
    let (ctx, _, _) = scene();
    // Long axis (z) turned to point along x: the box is now only 0.5 deep along the cast direction.
    let shape = QueryShape::Cuboid { half_extents: Vec3::new(0.5, 0.5, 3.0) };
    let rotated = ctx
        .cast_shape(shape, Vec3::ZERO, Quat::from_rotation_y(std::f32::consts::FRAC_PI_2), Vec3::NEG_Z, 50.0, &[], ALL)
        .unwrap();
    assert!((rotated.distance - 9.0).abs() < 1e-2, "{}", rotated.distance);
}

#[test]
fn casts_miss_out_of_range_away_or_sideways() {
    let (ctx, _, _) = scene();
    let sphere = QueryShape::Sphere { radius: 1.0 };
    assert!(cast(&ctx, sphere, Vec3::NEG_Z, 5.0, &[], ALL).is_none(), "too short");
    assert!(cast(&ctx, sphere, Vec3::Z, 50.0, &[], ALL).is_none(), "pointing away");
    assert!(cast(&ctx, sphere, Vec3::Y, 50.0, &[], ALL).is_none(), "pointing up");
    assert!(cast(&ctx, sphere, Vec3::ZERO, 50.0, &[], ALL).is_none(), "zero direction");
}

#[test]
fn excluded_entities_are_skipped_by_casts() {
    let (ctx, wall, _) = scene();
    let sphere = QueryShape::Sphere { radius: 1.0 };
    assert!(cast(&ctx, sphere, Vec3::NEG_Z, 50.0, &[wall], ALL).is_none());
}

#[test]
fn overlap_returns_exactly_the_colliders_inside_the_shape() {
    let (ctx, wall, ball) = scene();
    let at = |shape, position| ctx.overlap_shape(shape, position, Quat::IDENTITY, &[], ALL);

    assert!(at(QueryShape::Sphere { radius: 1.0 }, Vec3::ZERO).is_empty(), "nothing near the origin");
    assert_eq!(at(QueryShape::Sphere { radius: 3.0 }, Vec3::new(0.0, 0.0, -8.0)), vec![wall]);
    let both = at(QueryShape::Sphere { radius: 40.0 }, Vec3::ZERO);
    assert_eq!(both.len(), 2);
    assert!(both.contains(&wall) && both.contains(&ball));
    assert_eq!(
        at(QueryShape::Cuboid { half_extents: Vec3::splat(2.0) }, Vec3::new(20.0, 0.0, 0.0)),
        vec![ball]
    );
    assert_eq!(
        at(QueryShape::Capsule { half_height: 1.0, radius: 0.5 }, Vec3::new(0.0, 0.0, -9.2)),
        vec![wall]
    );
}

#[test]
fn overlap_any_matches_overlap_shape_and_honours_exclusion() {
    let (ctx, wall, _) = scene();
    let shape = QueryShape::Sphere { radius: 3.0 };
    let position = Vec3::new(0.0, 0.0, -8.0);
    assert!(ctx.overlap_any(shape, position, Quat::IDENTITY, &[], ALL));
    assert!(!ctx.overlap_any(shape, position, Quat::IDENTITY, &[wall], ALL));
    assert!(!ctx.overlap_any(shape, Vec3::ZERO, Quat::IDENTITY, &[], ALL));
    assert!(ctx.overlap_shape(shape, position, Quat::IDENTITY, &[wall], ALL).is_empty());
}

#[test]
fn query_masks_select_colliders_by_their_layer() {
    let (mut ctx, wall, ball) = scene();
    ctx.set_collider_layers(wall, CollisionLayers::new(0b01, ALL)).unwrap();
    ctx.set_collider_layers(ball, CollisionLayers::new(0b10, ALL)).unwrap();
    ctx.step(1.0 / 60.0);
    let big = QueryShape::Sphere { radius: 40.0 };

    assert_eq!(ctx.overlap_shape(big, Vec3::ZERO, Quat::IDENTITY, &[], 0b01), vec![wall]);
    assert_eq!(ctx.overlap_shape(big, Vec3::ZERO, Quat::IDENTITY, &[], 0b10), vec![ball]);
    assert_eq!(ctx.overlap_shape(big, Vec3::ZERO, Quat::IDENTITY, &[], 0b11).len(), 2);
    assert!(ctx.overlap_shape(big, Vec3::ZERO, Quat::IDENTITY, &[], 0b100).is_empty());

    let sphere = QueryShape::Sphere { radius: 1.0 };
    assert!(cast(&ctx, sphere, Vec3::NEG_Z, 50.0, &[], 0b10).is_none(), "the wall is on another layer");
    assert!(cast(&ctx, sphere, Vec3::NEG_Z, 50.0, &[], 0b01).is_some());
}

#[test]
fn setting_layers_on_an_unknown_entity_is_an_error() {
    let (mut ctx, _, _) = scene();
    let mut world = World::new();
    let stranger = world.spawn();
    assert!(ctx.set_collider_layers(stranger, CollisionLayers::default()).is_err());
}

#[test]
fn layer_defaults_and_constructors() {
    let all = CollisionLayers::default();
    assert_eq!((all.member_of, all.collides_with), (u32::MAX, u32::MAX));
    assert_eq!(CollisionLayers::ALL, all);
    let none = CollisionLayers::NONE;
    assert_eq!((none.member_of, none.collides_with), (0, 0));
    let custom = CollisionLayers::new(0b101, 0b010);
    assert_eq!((custom.member_of, custom.collides_with), (0b101, 0b010));
}

/// A ball dropped on a floor; returns its height after two seconds.
fn drop_height(ball_layers: Option<CollisionLayers>) -> f32 {
    let mut world = World::new();
    let floor = world.spawn();
    let ball = world.spawn();
    let mut ctx = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));
    ctx.add_rigid_body(
        floor,
        RigidBodyBuilder::fixed().translation(Vec3::new(0.0, -0.5, 0.0)),
        vec![ColliderBuilder::cuboid(50.0, 0.5, 50.0)],
    )
    .unwrap();
    let (handle, _) = ctx
        .add_rigid_body(
            ball,
            RigidBodyBuilder::dynamic().translation(Vec3::new(0.0, 3.0, 0.0)),
            vec![ColliderBuilder::ball(0.5)],
        )
        .unwrap();
    ctx.set_collider_layers(floor, CollisionLayers::new(0b100, ALL)).unwrap();
    if let Some(layers) = ball_layers {
        ctx.set_collider_layers(ball, layers).unwrap();
    }
    for _ in 0..120 {
        ctx.step(1.0 / 60.0);
    }
    ctx.get_rigid_body_transform(handle).unwrap().0.y
}

#[test]
fn layers_decide_whether_bodies_collide() {
    let lands = drop_height(None);
    assert!(lands > 0.3 && lands < 0.7, "default layers: the ball rests on the floor, y = {lands}");

    // The ball is on layer 1 and only collides with layer 2; the floor is on layer 3: no contact.
    let falls = drop_height(Some(CollisionLayers::new(0b001, 0b010)));
    assert!(falls < -5.0, "no collision: the ball falls through, y = {falls}");

    // The ball collides with layer 3 (the floor) again.
    let lands_again = drop_height(Some(CollisionLayers::new(0b001, 0b100)));
    assert!(lands_again > 0.3 && lands_again < 0.7, "y = {lands_again}");
}
