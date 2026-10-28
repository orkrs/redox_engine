//! Tests for `CombineRule` (PHY-05): how restitution of two surfaces is combined.

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::World;
use redox_math::Vec3;
use redox_physics::{CombineRule, PhysicsContext, PhysicsError, PhysicsMaterial};

/// Drops a ball of restitution 1.0 on a floor of restitution 0.0 and returns rebound height / drop height.
/// `ball_rule` and `floor_rule` are the restitution rules of the two colliders.
fn rebound_ratio(ball_rule: CombineRule, floor_rule: CombineRule) -> f32 {
    let mut world = World::new();
    let floor = world.spawn();
    let ball = world.spawn();
    let mut ctx = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));
    ctx.add_rigid_body(
        floor,
        RigidBodyBuilder::fixed().translation(Vec3::new(0.0, -0.5, 0.0)),
        vec![ColliderBuilder::cuboid(10.0, 0.5, 10.0)],
    )
    .unwrap();
    ctx.add_rigid_body(
        ball,
        RigidBodyBuilder::dynamic().translation(Vec3::new(0.0, 2.25, 0.0)),
        vec![ColliderBuilder::ball(0.25)],
    )
    .unwrap();
    let material = |restitution| PhysicsMaterial {
        friction: 0.5,
        restitution,
    };
    ctx.set_physics_material(floor, material(0.0)).unwrap();
    ctx.set_physics_material(ball, material(1.0)).unwrap();
    ctx.set_combine_rules(ball, CombineRule::Average, ball_rule)
        .unwrap();
    ctx.set_combine_rules(floor, CombineRule::Average, floor_rule)
        .unwrap();

    let dt = 1.0 / 120.0;
    let mut bounced = false;
    let mut previous_y = 2.25;
    let mut peak = 0.0f32;
    for _ in 0..(3.0 / dt) as usize {
        ctx.step(dt);
        let handle = ctx.get_body_handle(ball).unwrap();
        let y = ctx.get_rigid_body_transform(handle).unwrap().0.y;
        if y > previous_y && previous_y < 0.4 {
            bounced = true;
        }
        if bounced {
            peak = peak.max(y);
        }
        previous_y = y;
    }
    (peak - 0.25) / 2.0
}

#[test]
fn the_default_rule_averages_the_restitution() {
    let r = rebound_ratio(CombineRule::Average, CombineRule::Average);
    assert!(r > 0.1 && r < 0.45, "expected about 0.25, got {r}");
}

#[test]
fn max_rule_makes_the_ball_bounce_back_nearly_fully() {
    let r = rebound_ratio(CombineRule::Max, CombineRule::Average);
    assert!(r > 0.6, "expected close to 1.0, got {r}");
}

#[test]
fn min_rule_with_a_dead_floor_removes_the_bounce() {
    let r = rebound_ratio(CombineRule::Min, CombineRule::Average);
    assert!(r < 0.05, "expected no bounce, got {r}");
}

#[test]
fn a_stronger_rule_on_either_collider_wins() {
    let on_floor = rebound_ratio(CombineRule::Average, CombineRule::Max);
    let on_ball = rebound_ratio(CombineRule::Max, CombineRule::Average);
    assert!(on_floor > 0.6, "got {on_floor}");
    assert!((on_floor - on_ball).abs() < 0.1, "{on_floor} vs {on_ball}");
}

#[test]
fn unknown_entities_are_an_error() {
    let mut world = World::new();
    let mut ctx = PhysicsContext::new(Vec3::ZERO);
    let nobody = world.spawn();
    assert!(matches!(
        ctx.set_combine_rules(nobody, CombineRule::Max, CombineRule::Max),
        Err(PhysicsError::EntityNotFound)
    ));
}
