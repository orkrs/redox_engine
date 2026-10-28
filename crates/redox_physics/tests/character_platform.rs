//! Tests for carrying a character on a moving platform (PHY-03).
//!
//! The game plugin calls `sync_to_physics` before `tick_physics`; the tests do the same after moving the platform.

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::{Entity, World};
use redox_math::{Transform, Vec3};
use redox_physics::{
    CharacterController, Collider, Kinematic, PhysicsContext, RigidBody,
    sync::{sync_to_physics, tick_physics},
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

fn position(world: &World, e: Entity) -> Vec3 {
    world.get_component::<Transform>(e).unwrap().translation
}

/// A 4x4 kinematic platform with its top at y = 0 and a character standing on it.
fn platform_with_player() -> (World, Entity, Entity) {
    let mut world = World::new();
    world.insert_resource(PhysicsContext::new(Vec3::ZERO));
    let platform = spawn_body(
        &mut world,
        Vec3::new(0.0, -0.25, 0.0),
        RigidBodyBuilder::kinematic_position_based(),
        ColliderBuilder::cuboid(2.0, 0.25, 2.0),
    );
    world.add_component(platform, Kinematic);
    let player = spawn_body(
        &mut world,
        Vec3::new(0.0, 0.75, 0.0),
        RigidBodyBuilder::kinematic_position_based(),
        ColliderBuilder::capsule_y(0.4, 0.3),
    );
    world.add_component(player, Kinematic);
    world.add_component(player, CharacterController::default());
    for _ in 0..30 {
        sync_to_physics(&mut world);
        tick_physics(&mut world, DT);
    }
    (world, platform, player)
}

fn move_platform(world: &mut World, platform: Entity, dx: f32) {
    world
        .get_component_mut::<Transform>(platform)
        .unwrap()
        .translation
        .x += dx;
}

#[test]
fn standing_on_a_moving_platform_carries_the_character() {
    let (mut world, platform, player) = platform_with_player();
    let start = position(&world, player);
    for _ in 0..60 {
        move_platform(&mut world, platform, 1.0 * DT); // 1 unit/s
        sync_to_physics(&mut world);
        tick_physics(&mut world, DT);
    }
    let end = position(&world, player);
    assert!(
        (end.x - start.x - 1.0).abs() < 0.15,
        "moved {} of 1.0",
        end.x - start.x
    );
    assert!((end.y - start.y).abs() < 0.05, "stayed at the same height");
}

#[test]
fn leaving_the_platform_stops_the_carry() {
    let (mut world, platform, player) = platform_with_player();
    for _ in 0..30 {
        move_platform(&mut world, platform, 1.0 * DT);
        sync_to_physics(&mut world);
        tick_physics(&mut world, DT);
    }
    // Walk off sideways (z) at 2 units/s while the platform keeps moving along x.
    for _ in 0..90 {
        world
            .get_component_mut::<CharacterController>(player)
            .unwrap()
            .desired_translation = Vec3::new(0.0, 0.0, 2.0 * DT);
        move_platform(&mut world, platform, 1.0 * DT);
        sync_to_physics(&mut world);
        tick_physics(&mut world, DT);
    }
    let before = position(&world, player).x;
    for _ in 0..20 {
        move_platform(&mut world, platform, 1.0 * DT);
        sync_to_physics(&mut world);
        tick_physics(&mut world, DT);
    }
    let after = position(&world, player).x;
    assert!(
        (after - before).abs() < 0.05,
        "x kept changing after leaving: {before} -> {after}"
    );
}

/// Walking across a platform that is a kinematic body must work like walking on static ground
/// (Rapier's own controller froze the character there before this was handled).
#[test]
fn walking_on_a_still_kinematic_platform_is_not_blocked() {
    let (mut world, _platform, player) = platform_with_player();
    for _ in 0..60 {
        world
            .get_component_mut::<CharacterController>(player)
            .unwrap()
            .desired_translation = Vec3::new(0.0, 0.0, 1.0 * DT);
        sync_to_physics(&mut world);
        tick_physics(&mut world, DT);
    }
    let z = position(&world, player).z;
    assert!((z - 1.0).abs() < 0.1, "walked {z} of 1.0 units");
}
