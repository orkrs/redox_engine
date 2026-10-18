//! Specification (tests first) for resetting the `stay` opt-in when a body is removed (plan item B3,
//! known issue).
//!
//! Today `contact_stay_entities` (in `context.rs`) keeps an entity after `remove_rigid_body`, so a body
//! that is later added again for the same entity silently reports stays without a new opt-in.
//! Required: `remove_rigid_body` also forgets the entity's stay opt-in (one line next to
//! `forget_joints_of` / `applied_layers.remove`). The other opt-in, `set_contact_events`, lives on the
//! colliders and disappears with them, so nothing to do there.

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::{Entity, World};
use redox_math::Vec3;
use redox_physics::PhysicsContext;

const DT: f32 = 1.0 / 60.0;

fn add_floor(ctx: &mut PhysicsContext, floor: Entity) {
    ctx.add_rigid_body(
        floor,
        RigidBodyBuilder::fixed().translation(Vec3::new(0.0, -0.5, 0.0)),
        vec![ColliderBuilder::cuboid(50.0, 0.5, 50.0)],
    )
    .unwrap();
}

fn add_ball(ctx: &mut PhysicsContext, ball: Entity) {
    ctx.add_rigid_body(
        ball,
        RigidBodyBuilder::dynamic().translation(Vec3::new(0.0, 0.52, 0.0)),
        vec![ColliderBuilder::ball(0.5)],
    )
    .unwrap();
}

#[test]
fn a_re_added_body_must_opt_in_to_stay_again() {
    let mut world = World::new();
    let floor = world.spawn();
    let ball = world.spawn();
    let mut ctx = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));
    add_floor(&mut ctx, floor);
    add_ball(&mut ctx, ball);
    ctx.set_contact_stay(ball, true).unwrap();
    for _ in 0..30 {
        ctx.step(DT);
    }
    assert!(
        !ctx.drain_contact_stays().is_empty(),
        "opted in: stays flow"
    );

    ctx.remove_rigid_body(ball).unwrap();
    add_ball(&mut ctx, ball);
    for _ in 0..30 {
        ctx.step(DT);
    }
    assert!(
        ctx.drain_contact_stays().is_empty(),
        "the opt-in did not survive the removal"
    );
}

#[test]
fn opting_in_again_after_re_adding_works() {
    let mut world = World::new();
    let floor = world.spawn();
    let ball = world.spawn();
    let mut ctx = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));
    add_floor(&mut ctx, floor);
    add_ball(&mut ctx, ball);
    ctx.set_contact_stay(ball, true).unwrap();
    ctx.remove_rigid_body(ball).unwrap();
    add_ball(&mut ctx, ball);
    ctx.set_contact_stay(ball, true).unwrap();
    for _ in 0..30 {
        ctx.step(DT);
    }
    assert!(!ctx.drain_contact_stays().is_empty());
}

#[test]
fn removing_an_entity_that_never_opted_in_still_works() {
    let mut world = World::new();
    let ball = world.spawn();
    let mut ctx = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));
    add_ball(&mut ctx, ball);
    ctx.remove_rigid_body(ball).unwrap();
    assert!(ctx.remove_rigid_body(ball).is_err(), "second removal fails");
}
