//! Character grounding on slopes, ledges, stairs and walls (PHY-06, step 2).
//!
//! The game plugin calls `sync_to_physics` before `tick_physics`; the tests do the same.

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::{Entity, World};
use redox_math::{Quat, Transform, Vec3};
use redox_physics::{
    CharacterController, Collider, Kinematic, PhysicsContext, RigidBody,
    components::CharacterSettings,
    sync::{sync_to_physics, tick_physics},
};

const DT: f32 = 1.0 / 60.0;
/// Slope of the test ramp in radians (40 degrees, below the 50 degree climb limit).
const SLOPE: f32 = 40.0 * std::f32::consts::PI / 180.0;
/// Distance from the capsule centre down to its lowest point: half height plus radius.
const CAPSULE_CENTRE_ABOVE_FEET: f32 = 0.7;

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

fn spawn_static_box(world: &mut World, centre: Vec3, half: Vec3) {
    spawn_body(
        world,
        centre,
        RigidBodyBuilder::fixed(),
        ColliderBuilder::cuboid(half.x, half.y, half.z),
    );
}

/// Ramp whose surface passes through the origin and falls towards +X at `SLOPE`.
fn spawn_ramp(world: &mut World) {
    let normal = Vec3::new(SLOPE.sin(), SLOPE.cos(), 0.0);
    let centre = -normal * 0.25;
    let ramp = spawn_body(
        world,
        centre,
        RigidBodyBuilder::fixed(),
        ColliderBuilder::cuboid(30.0, 0.25, 5.0),
    );
    let handle = world.get_component::<RigidBody>(ramp).unwrap().handle;
    let mut phys = world.remove_resource::<PhysicsContext>().unwrap();
    phys.set_rigid_body_transform(handle, centre, Quat::from_rotation_z(-SLOPE));
    world.insert_resource(phys);
}

fn spawn_player(world: &mut World, at: Vec3, settings: CharacterSettings) -> Entity {
    let player = spawn_body(
        world,
        at,
        RigidBodyBuilder::kinematic_position_based(),
        ColliderBuilder::capsule_y(0.4, 0.3),
    );
    world.add_component(player, Kinematic);
    world.add_component(
        player,
        CharacterController {
            settings,
            ..Default::default()
        },
    );
    player
}

fn steep_but_walkable() -> CharacterSettings {
    CharacterSettings {
        max_slope_angle: 50.0_f32.to_radians(),
        ..Default::default()
    }
}

fn position(world: &World, e: Entity) -> Vec3 {
    world.get_component::<Transform>(e).unwrap().translation
}

fn grounded(world: &World, e: Entity) -> bool {
    world.get_component::<CharacterController>(e).unwrap().grounded
}

/// Walks the character at `speed` along +X for `ticks` steps; returns how many ticks ended in the air.
fn walk_counting_airborne(world: &mut World, e: Entity, speed: f32, ticks: usize) -> usize {
    let mut airborne = 0;
    for _ in 0..ticks {
        world
            .get_component_mut::<CharacterController>(e)
            .unwrap()
            .desired_translation = Vec3::new(speed * DT, 0.0, 0.0);
        sync_to_physics(world);
        tick_physics(world, DT);
        if !grounded(world, e) {
            airborne += 1;
        }
    }
    airborne
}

#[test]
fn walking_down_a_steep_slope_stays_grounded() {
    let mut world = World::new();
    world.insert_resource(PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0)));
    spawn_ramp(&mut world);
    // Start 3 m up the ramp, feet just above the surface (height = 3 * tan 40 degrees).
    let start = Vec3::new(-3.0, 3.0 * SLOPE.tan() + CAPSULE_CENTRE_ABOVE_FEET + 0.02, 0.0);
    let player = spawn_player(&mut world, start, steep_but_walkable());

    let ticks = 180; // 3 s
    let airborne = walk_counting_airborne(&mut world, player, 3.0, ticks);
    let share = airborne as f32 / ticks as f32;
    assert!(share <= 0.05, "in the air {airborne} of {ticks} ticks ({share:.3})");
}

#[test]
fn a_ledge_still_lets_the_character_fall() {
    let mut world = World::new();
    world.insert_resource(PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0)));
    // Upper ledge: top at y = 0 for x in [-10, 0].
    spawn_static_box(&mut world, Vec3::new(-5.0, -0.25, 0.0), Vec3::new(5.0, 0.25, 5.0));
    // Lower floor: top at y = -2 for x in [0, 20], two metres below the ledge.
    spawn_static_box(&mut world, Vec3::new(10.0, -2.25, 0.0), Vec3::new(10.0, 0.25, 5.0));
    let player = spawn_player(
        &mut world,
        Vec3::new(-3.0, 0.72, 0.0),
        CharacterSettings::default(),
    );

    walk_counting_airborne(&mut world, player, 3.0, 120); // 2 s, walks off the edge at x = 0
    let y = position(&world, player).y;
    assert!(y < -0.5, "stayed on the ledge level, y={y}");
}

#[test]
fn jumping_still_leaves_the_ground_on_a_slope() {
    let mut world = World::new();
    world.insert_resource(PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0)));
    spawn_ramp(&mut world);
    let start = Vec3::new(-3.0, 3.0 * SLOPE.tan() + CAPSULE_CENTRE_ABOVE_FEET + 0.02, 0.0);
    let player = spawn_player(&mut world, start, steep_but_walkable());
    for _ in 0..20 {
        sync_to_physics(&mut world);
        tick_physics(&mut world, DT);
    }
    assert!(grounded(&world, player), "expected to stand on the ramp before jumping");
    let rest_y = position(&world, player).y;

    assert!(
        world
            .get_component_mut::<CharacterController>(player)
            .unwrap()
            .jump(),
        "jump refused on the ground"
    );
    let mut highest = rest_y;
    for _ in 0..10 {
        sync_to_physics(&mut world);
        tick_physics(&mut world, DT);
        highest = highest.max(position(&world, player).y);
    }
    assert!(
        highest > rest_y + 0.2,
        "jump on a slope barely rose: {rest_y} -> {highest}"
    );
}

#[test]
fn walking_down_stairs_keeps_contact() {
    let mut world = World::new();
    world.insert_resource(PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0)));
    // Flat ground for x < 0 (top at y = 0).
    spawn_static_box(&mut world, Vec3::new(-5.0, -0.25, 0.0), Vec3::new(5.0, 0.25, 5.0));
    // Ten treads, each 0.5 m long and 0.2 m lower than the one before.
    for i in 0..10 {
        let top = -0.2 * (i as f32 + 1.0);
        let centre_x = 0.25 + 0.5 * i as f32;
        spawn_static_box(
            &mut world,
            Vec3::new(centre_x, top - 0.25, 0.0),
            Vec3::new(0.25, 0.25, 5.0),
        );
    }
    // Lower floor past the stairs (top at y = -2).
    spawn_static_box(&mut world, Vec3::new(15.0, -2.25, 0.0), Vec3::new(10.0, 0.25, 5.0));
    let player = spawn_player(
        &mut world,
        Vec3::new(-3.0, 0.72, 0.0),
        CharacterSettings::default(),
    );

    let ticks = 240; // 4 s at 2 m/s covers the whole staircase
    let mut grounded_ticks = 0;
    for _ in 0..ticks {
        world
            .get_component_mut::<CharacterController>(player)
            .unwrap()
            .desired_translation = Vec3::new(2.0 * DT, 0.0, 0.0);
        sync_to_physics(&mut world);
        tick_physics(&mut world, DT);
        if grounded(&world, player) {
            grounded_ticks += 1;
        }
    }
    let share = grounded_ticks as f32 / ticks as f32;
    assert!(share >= 0.9, "grounded on {share:.3} of ticks, expected at least 0.9");
}

#[test]
fn the_snap_does_not_pull_the_character_up_a_wall() {
    let mut world = World::new();
    world.insert_resource(PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0)));
    spawn_static_box(&mut world, Vec3::new(0.0, -0.25, 0.0), Vec3::new(20.0, 0.25, 5.0));
    // Wall from x = 1.0 to 1.5, rising from y = 0 to 4: far above the step height.
    spawn_static_box(&mut world, Vec3::new(1.25, 2.0, 0.0), Vec3::new(0.25, 2.0, 5.0));
    let start_y = 0.72;
    let player = spawn_player(
        &mut world,
        Vec3::new(0.0, start_y, 0.0),
        CharacterSettings::default(),
    );

    let mut highest = start_y;
    for _ in 0..60 {
        world
            .get_component_mut::<CharacterController>(player)
            .unwrap()
            .desired_translation = Vec3::new(3.0 * DT, 0.0, 0.0);
        sync_to_physics(&mut world);
        tick_physics(&mut world, DT);
        highest = highest.max(position(&world, player).y);
    }
    assert!(highest < start_y + 0.05, "climbed the wall: y rose to {highest}");
    assert!(position(&world, player).x < 1.0, "went through the wall");
}
