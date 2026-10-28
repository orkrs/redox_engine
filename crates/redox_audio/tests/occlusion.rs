//! Tests for per-source sound occlusion (AUD-03): hits map to a coefficient, each emitter has its own value and
//! trigger volumes are not walls.

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_audio::{
    AudioContext, AudioListener, SpatialAudioEmitter, occlusion_from_hits, occlusion_raycast_system,
};
use redox_ecs::{Entity, World};
use redox_math::Vec3;
use redox_physics::PhysicsContext;

fn world_with_listener() -> World {
    let mut world = World::new();
    world.insert_resource(PhysicsContext::new(Vec3::ZERO));
    world.insert_resource(AudioContext::new());
    let listener = world.spawn();
    world.add_component(listener, AudioListener::default());
    world
}

fn add_emitter(world: &mut World, at: Vec3) -> Entity {
    let e = world.spawn();
    world.add_component(e, SpatialAudioEmitter::new(at));
    e
}

fn add_wall(world: &mut World, at: Vec3) {
    let e = world.spawn();
    let mut ctx = world.remove_resource::<PhysicsContext>().unwrap();
    ctx.add_rigid_body(
        e,
        RigidBodyBuilder::fixed().translation(at),
        vec![ColliderBuilder::cuboid(5.0, 5.0, 0.2)],
    )
    .unwrap();
    world.insert_resource(ctx);
}

fn settle(world: &mut World) {
    world
        .get_resource_mut::<PhysicsContext>()
        .unwrap()
        .step(1.0 / 60.0); // builds the broad phase used by queries
}

fn coefficient(world: &World, e: Entity) -> f32 {
    world
        .get_component::<SpatialAudioEmitter>(e)
        .unwrap()
        .occlusion_coefficient
}

#[test]
fn hits_map_to_the_documented_occlusion() {
    assert_eq!(occlusion_from_hits(0), 0.0);
    assert_eq!(occlusion_from_hits(1), 0.8);
    assert!(occlusion_from_hits(2) > occlusion_from_hits(1));
    assert_eq!(occlusion_from_hits(3), occlusion_from_hits(10));
}

#[test]
fn a_clear_line_leaves_the_emitter_untouched() {
    let mut world = world_with_listener();
    let e = add_emitter(&mut world, Vec3::new(0.0, 0.0, -10.0));
    settle(&mut world);
    occlusion_raycast_system(&mut world);
    assert_eq!(coefficient(&world, e), 0.0);
    let ctx = world.get_resource::<AudioContext>().unwrap();
    assert_eq!(ctx.get_emitter_occlusion(e), 0.0);
}

#[test]
fn two_walls_occlude_more_than_one() {
    let mut world = world_with_listener();
    let e = add_emitter(&mut world, Vec3::new(0.0, 0.0, -10.0));
    add_wall(&mut world, Vec3::new(0.0, 0.0, -3.0));
    settle(&mut world);
    occlusion_raycast_system(&mut world);
    assert_eq!(coefficient(&world, e), occlusion_from_hits(1));

    add_wall(&mut world, Vec3::new(0.0, 0.0, -6.0));
    settle(&mut world);
    occlusion_raycast_system(&mut world);
    assert_eq!(coefficient(&world, e), occlusion_from_hits(2));
}

#[test]
fn one_blocked_emitter_does_not_muffle_the_rest() {
    let mut world = world_with_listener();
    let blocked = add_emitter(&mut world, Vec3::new(0.0, 0.0, -10.0));
    let clear = add_emitter(&mut world, Vec3::new(8.0, 0.0, 10.0));
    add_wall(&mut world, Vec3::new(0.0, 0.0, -3.0));
    settle(&mut world);
    occlusion_raycast_system(&mut world);
    assert_eq!(coefficient(&world, clear), 0.0);
    let ctx = world.get_resource::<AudioContext>().unwrap();
    assert_eq!(ctx.emitter_volume(clear, 1.0), 1.0);
    assert!(ctx.emitter_volume(blocked, 1.0) < 1.0);
}

#[test]
fn a_trigger_between_listener_and_emitter_does_not_occlude() {
    let mut world = world_with_listener();
    let e = add_emitter(&mut world, Vec3::new(0.0, 0.0, -10.0));
    let trigger = world.spawn();
    let mut ctx = world.remove_resource::<PhysicsContext>().unwrap();
    ctx.add_trigger(
        trigger,
        Vec3::new(0.0, 0.0, -5.0),
        ColliderBuilder::ball(1.0),
    )
    .unwrap();
    world.insert_resource(ctx);
    settle(&mut world);
    occlusion_raycast_system(&mut world);
    assert_eq!(coefficient(&world, e), 0.0);
}
