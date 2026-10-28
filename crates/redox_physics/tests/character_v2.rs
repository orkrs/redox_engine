//! Specification (tests first) for the character controller 2.0 (plan items PHY-01, PHY-02).
//!
//! Both tests use only the API that exists today, so they compile now and are `#[ignore]`d because
//! they fail until the tasks are done. Remove the `#[ignore]` of a test in the task that implements
//! it. The full task text is in `docs/engineering/specs/PHY-01.md` and `PHY-02.md`.

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::{Entity, World};
use redox_math::{Transform, Vec3};
use redox_physics::{
    CharacterController, Collider, Kinematic, PhysicsContext, RigidBody, sync::tick_physics,
};

const DT: f32 = 1.0 / 60.0;

fn spawn_body(
    world: &mut World,
    translation: Vec3,
    body: RigidBodyBuilder,
    collider: ColliderBuilder,
) -> Entity {
    let e = world.spawn();
    world.add_component(e, Transform::from_translation(translation));
    let mut phys = world.remove_resource::<PhysicsContext>().unwrap();
    let (bh, cols) = phys
        .add_rigid_body(e, body.translation(translation), vec![collider])
        .unwrap();
    world.insert_resource(phys);
    world.add_component(e, RigidBody { handle: bh });
    world.add_component(e, Collider { handle: cols[0] });
    e
}

fn spawn_player(world: &mut World, at: Vec3) -> Entity {
    let player = spawn_body(
        world,
        at,
        RigidBodyBuilder::kinematic_position_based(),
        ColliderBuilder::capsule_y(0.4, 0.3),
    );
    world.add_component(player, Kinematic);
    world.add_component(player, CharacterController::default());
    player
}

fn position(world: &World, e: Entity) -> Vec3 {
    world.get_component::<Transform>(e).unwrap().translation
}

/// Gravity must accelerate the fall: after one second with `gravity = 20` the character has dropped
/// about `20 / 2 = 10` units (semi-implicit Euler gives 10.17). Today every step adds the same
/// `gravity * dt` drop, so the fall is linear and the character drops 20 units in the same second.
#[test]
fn free_fall_accelerates_like_real_gravity() {
    let mut world = World::new();
    world.insert_resource(PhysicsContext::new(Vec3::ZERO));
    spawn_body(
        &mut world,
        Vec3::new(0.0, -100.5, 0.0),
        RigidBodyBuilder::fixed(),
        ColliderBuilder::cuboid(10.0, 0.5, 10.0),
    );
    let player = spawn_player(&mut world, Vec3::new(0.0, 50.0, 0.0));
    for _ in 0..60 {
        tick_physics(&mut world, DT);
    }
    let fallen = 50.0 - position(&world, player).y;
    assert!(
        (9.0..=11.5).contains(&fallen),
        "fell {fallen} units in one second, expected about 10"
    );
}

/// A 0.2 high step must be climbed: the capsule keeps walking and ends up standing on top of it.
/// Without autostep (today's default) the capsule stops in front of the step face.
#[test]
#[ignore = "TODO: PHY-02"]
fn walking_into_a_low_step_climbs_it() {
    let mut world = World::new();
    world.insert_resource(PhysicsContext::new(Vec3::ZERO));
    spawn_body(
        &mut world,
        Vec3::new(0.0, -0.5, 0.0),
        RigidBodyBuilder::fixed(),
        ColliderBuilder::cuboid(20.0, 0.5, 5.0),
    );
    // Step: top at y = 0.2, spans x from 1 to 5.
    spawn_body(
        &mut world,
        Vec3::new(3.0, 0.1, 0.0),
        RigidBodyBuilder::fixed(),
        ColliderBuilder::cuboid(2.0, 0.1, 2.0),
    );
    let player = spawn_player(&mut world, Vec3::new(0.0, 0.75, 0.0));
    for _ in 0..120 {
        world
            .get_component_mut::<CharacterController>(player)
            .unwrap()
            .desired_translation = Vec3::new(3.0 * DT, 0.0, 0.0);
        tick_physics(&mut world, DT);
    }
    let p = position(&world, player);
    assert!(p.x > 2.5, "stopped in front of the step, x = {}", p.x);
    assert!(
        (0.85..=1.1).contains(&p.y),
        "should stand on the step (capsule centre about 0.9), y = {}",
        p.y
    );
}

#[test]
fn jump_rises_then_lands_back_on_the_floor() {
    let mut world = World::new();
    world.insert_resource(PhysicsContext::new(Vec3::ZERO));
    spawn_body(
        &mut world,
        Vec3::new(0.0, -0.5, 0.0),
        RigidBodyBuilder::fixed(),
        ColliderBuilder::cuboid(10.0, 0.5, 10.0),
    );
    let player = spawn_player(&mut world, Vec3::new(0.0, 0.75, 0.0));
    for _ in 0..30 {
        tick_physics(&mut world, DT);
    } // settle
    assert!(
        world
            .get_component::<CharacterController>(player)
            .unwrap()
            .grounded
    );
    let rest = position(&world, player).y;
    assert!(
        world
            .get_component_mut::<CharacterController>(player)
            .unwrap()
            .jump()
    );
    let mut peak = rest;
    for _ in 0..30 {
        tick_physics(&mut world, DT);
        peak = peak.max(position(&world, player).y);
    }
    // v^2 / 2g = 49 / 40 = 1.225
    assert!((peak - rest - 1.2).abs() < 0.2, "peak rise {}", peak - rest);
    for _ in 0..120 {
        tick_physics(&mut world, DT);
    }
    assert!((position(&world, player).y - rest).abs() < 0.05);
    assert!(
        world
            .get_component::<CharacterController>(player)
            .unwrap()
            .grounded
    );
}

#[test]
fn jump_in_the_air_is_refused() {
    let mut cc = CharacterController::default();
    assert!(!cc.jump(), "not grounded yet");
    cc.grounded = true;
    assert!(cc.jump());
    assert!(!cc.jump(), "already left the ground");
}

#[test]
fn a_low_ceiling_stops_the_rise() {
    let mut world = World::new();
    world.insert_resource(PhysicsContext::new(Vec3::ZERO));
    spawn_body(
        &mut world,
        Vec3::new(0.0, -0.5, 0.0),
        RigidBodyBuilder::fixed(),
        ColliderBuilder::cuboid(10.0, 0.5, 10.0),
    );
    // underside of the ceiling at y = 1.6
    spawn_body(
        &mut world,
        Vec3::new(0.0, 2.1, 0.0),
        RigidBodyBuilder::fixed(),
        ColliderBuilder::cuboid(10.0, 0.5, 10.0),
    );
    let player = spawn_player(&mut world, Vec3::new(0.0, 0.75, 0.0));
    for _ in 0..30 {
        tick_physics(&mut world, DT);
    }
    world
        .get_component_mut::<CharacterController>(player)
        .unwrap()
        .jump();
    for _ in 0..20 {
        tick_physics(&mut world, DT);
    }
    let top = position(&world, player).y + 0.7; // capsule half height
    assert!(top <= 1.65, "head went through the ceiling: {top}");
}
