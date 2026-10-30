//! Tests for carrying a character on a moving platform (PHY-03).
//!
//! The game plugin calls `sync_to_physics` before `tick_physics`; the tests do the same after moving the platform.

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::{Entity, World};
use redox_math::{Quat, Transform, Vec3};
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

/// A 6x6 kinematic platform centred on the origin with a character standing at radius 2 on it.
fn turning_platform_with_player() -> (World, Entity, Entity) {
    let mut world = World::new();
    world.insert_resource(PhysicsContext::new(Vec3::ZERO));
    let platform = spawn_body(
        &mut world,
        Vec3::new(0.0, -0.25, 0.0),
        RigidBodyBuilder::kinematic_position_based(),
        ColliderBuilder::cuboid(3.0, 0.25, 3.0),
    );
    world.add_component(platform, Kinematic);
    let player = spawn_body(
        &mut world,
        Vec3::new(2.0, 0.75, 0.0),
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

/// Sets the platform's pose for this step: its translation and a turn of `angle` about +Y.
fn set_platform_pose(world: &mut World, platform: Entity, translation: Vec3, angle: f32) {
    let t = world.get_component_mut::<Transform>(platform).unwrap();
    t.translation = translation;
    t.rotation = Quat::from_rotation_y(angle);
}

/// The character's position in the platform's own frame.
fn local_point(world: &World, platform: Entity, player: Entity) -> Vec3 {
    let p = world.get_component::<Transform>(platform).unwrap();
    p.rotation.inverse() * (position(world, player) - p.translation)
}

#[test]
fn a_character_on_a_rotating_platform_moves_along_the_circle() {
    let (mut world, platform, player) = turning_platform_with_player();
    let origin = Vec3::new(0.0, -0.25, 0.0);
    let mut angle = 0.0;
    for _ in 0..60 {
        angle += DT; // 1 rad/s
        set_platform_pose(&mut world, platform, origin, angle);
        sync_to_physics(&mut world);
        tick_physics(&mut world, DT);
    }
    let p = position(&world, player);
    // A turn about +Y maps the offset (r cos a, 0, -r sin a) from the axis.
    let measured = (-p.z).atan2(p.x);
    let radius = (p.x * p.x + p.z * p.z).sqrt();
    assert!(
        (measured - 1.0).abs() < 0.1,
        "angle on the circle {measured}"
    );
    assert!((radius - 2.0).abs() < 0.05, "distance to the axis {radius}");
}

#[test]
fn the_character_stays_at_the_same_point_of_the_platform_in_its_local_frame() {
    let (mut world, platform, player) = turning_platform_with_player();
    let origin = Vec3::new(0.0, -0.25, 0.0);
    let start = local_point(&world, platform, player);
    let mut angle = 0.0;
    let mut drift_at_half = 0.0;
    for step in 0..180 {
        // 3 s
        angle += DT;
        set_platform_pose(&mut world, platform, origin, angle);
        sync_to_physics(&mut world);
        tick_physics(&mut world, DT);
        if step == 89 {
            drift_at_half = (local_point(&world, platform, player) - start).length();
        }
    }
    let drift = (local_point(&world, platform, player) - start).length();
    // PHY-06: the carry is applied outside the controller's collision response (Rapier's controller
    // clipped it against the platform's own collider on some ticks, ~5 % of the turn). The spec asks for 5 cm.
    assert!(drift <= 0.05, "drifted {drift} in the platform frame");
    assert!(
        drift - drift_at_half <= 0.02,
        "the drift grows faster than measured: {drift_at_half} after 1.5 s, {drift} after 3 s"
    );
}

/// Regression for the stale-pose carry (PHY-06): a single large turn of the platform must move the
/// character in the same tick. Before the fix the carry used the platform's previous collider pose,
/// so the character stayed about 1 m behind after a 0.5 rad turn.
#[test]
fn a_platform_turn_is_carried_in_the_same_tick() {
    let (mut world, platform, player) = turning_platform_with_player();
    let origin = Vec3::new(0.0, -0.25, 0.0);
    let local = local_point(&world, platform, player);
    set_platform_pose(&mut world, platform, origin, 0.5);
    sync_to_physics(&mut world);
    tick_physics(&mut world, DT);
    let expected = origin + Quat::from_rotation_y(0.5) * local;
    let p = position(&world, player);
    let error = ((p.x - expected.x).powi(2) + (p.z - expected.z).powi(2)).sqrt();
    assert!(
        error < 0.15,
        "character {p:?} is off the expected point {expected:?} by {error}"
    );
}

#[test]
fn a_platform_that_rotates_and_translates_carries_the_character_with_both() {
    let (mut world, platform, player) = turning_platform_with_player();
    let origin = Vec3::new(0.0, -0.25, 0.0);
    let local = local_point(&world, platform, player);
    let mut angle = 0.0;
    let mut shift = 0.0;
    for _ in 0..60 {
        angle += DT; // 1 rad/s
        shift += DT; // 1 unit/s along +X
        set_platform_pose(
            &mut world,
            platform,
            origin + Vec3::new(shift, 0.0, 0.0),
            angle,
        );
        sync_to_physics(&mut world);
        tick_physics(&mut world, DT);
    }
    let platform_origin = origin + Vec3::new(shift, 0.0, 0.0);
    let expected = platform_origin + Quat::from_rotation_y(angle) * local;
    let error = (position(&world, player) - expected).length();
    // Bounded by the three-tick lag of the carry (measured 0.12 m here).
    assert!(error < 0.2, "off the expected point by {error}");
}

#[test]
fn inheriting_yaw_turns_the_character_with_the_platform() {
    let (mut world, platform, player) = turning_platform_with_player();
    world
        .get_component_mut::<CharacterController>(player)
        .unwrap()
        .settings
        .inherit_platform_yaw = true;
    let origin = Vec3::new(0.0, -0.25, 0.0);
    let mut angle = 0.0;
    for _ in 0..60 {
        angle += DT; // ends at 1 rad
        set_platform_pose(&mut world, platform, origin, angle);
        sync_to_physics(&mut world);
        tick_physics(&mut world, DT);
    }
    let rotation = world.get_component::<Transform>(player).unwrap().rotation;
    let forward = rotation * Vec3::Z;
    let yaw = forward.x.atan2(forward.z);
    assert!(
        (yaw - 1.0).abs() < 0.05,
        "character yaw {yaw}, platform turned 1.0"
    );
}

#[test]
fn without_the_flag_the_character_keeps_its_heading_on_a_turning_platform() {
    let (mut world, platform, player) = turning_platform_with_player();
    let origin = Vec3::new(0.0, -0.25, 0.0);
    let mut angle = 0.0;
    for _ in 0..60 {
        angle += DT;
        set_platform_pose(&mut world, platform, origin, angle);
        sync_to_physics(&mut world);
        tick_physics(&mut world, DT);
    }
    let rotation = world.get_component::<Transform>(player).unwrap().rotation;
    let forward = rotation * Vec3::Z;
    let yaw = forward.x.atan2(forward.z);
    assert!(yaw.abs() < 0.05, "character turned without the flag: {yaw}");
}
