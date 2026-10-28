//! Specification (tests first) for AUD-03, part A: `cast_ray_excluding` must ignore sensors.
//!
//! Sound occlusion casts a ray from the listener to an emitter with `PhysicsContext::cast_ray_excluding`.
//! A trigger volume (sensor collider) is not a wall: sound passes through it. The test is `#[ignore]`d
//! until AUD-03 makes the query skip sensors; remove the `#[ignore]` in that task.

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::World;
use redox_math::Vec3;
use redox_physics::PhysicsContext;

fn context_with(sensor: bool) -> PhysicsContext {
    let mut world = World::new();
    let between = world.spawn();
    let mut ctx = PhysicsContext::new(Vec3::ZERO);
    if sensor {
        ctx.add_trigger(
            between,
            Vec3::new(0.0, 0.0, -5.0),
            ColliderBuilder::ball(1.0),
        )
        .unwrap();
    } else {
        ctx.add_rigid_body(
            between,
            RigidBodyBuilder::fixed().translation(Vec3::new(0.0, 0.0, -5.0)),
            vec![ColliderBuilder::ball(1.0)],
        )
        .unwrap();
    }
    ctx.step(1.0 / 60.0); // builds the broad phase used by queries
    ctx
}

#[test]
fn a_solid_object_between_listener_and_emitter_blocks_the_ray() {
    let ctx = context_with(false);
    let hit = ctx.cast_ray_excluding(Vec3::ZERO, Vec3::new(0.0, 0.0, -10.0), &[]);
    assert!(hit.is_some(), "a solid ball is a wall for sound");
}

#[test]
#[ignore = "TODO: AUD-03"]
fn a_trigger_volume_does_not_block_the_ray() {
    let ctx = context_with(true);
    let hit = ctx.cast_ray_excluding(Vec3::ZERO, Vec3::new(0.0, 0.0, -10.0), &[]);
    assert!(hit.is_none(), "sound passes through a sensor, got {hit:?}");
}
