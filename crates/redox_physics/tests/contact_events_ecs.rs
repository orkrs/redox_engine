//! `tick_physics` delivers contact events into the ECS `Events<ContactEvent>` resource.

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::{Events, World};
use redox_math::Vec3;
use redox_physics::{ContactEvent, PhysicsContext, sync::tick_physics};

const DT: f32 = 1.0 / 60.0;

#[test]
fn tick_physics_sends_one_enter_for_a_landing_ball_into_the_ecs() {
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
    ctx.add_rigid_body(
        ball,
        RigidBodyBuilder::dynamic().translation(Vec3::new(0.0, 2.5, 0.0)),
        vec![ColliderBuilder::ball(0.5)],
    )
    .unwrap();
    ctx.set_contact_events(ball, true).unwrap();
    world.insert_resource(ctx);
    world.insert_resource(Events::<ContactEvent>::new());

    for _ in 0..120 {
        tick_physics(&mut world, DT);
    }

    let events = world.get_resource::<Events<ContactEvent>>().unwrap();
    let enters = events
        .iter()
        .filter(|e| matches!(e, ContactEvent::Enter(_)))
        .count();
    assert_eq!(
        enters,
        1,
        "one landing: {:?}",
        events.iter().collect::<Vec<_>>()
    );
}

#[test]
fn tick_physics_without_an_events_resource_still_runs() {
    let mut world = World::new();
    let ball = world.spawn();
    let mut ctx = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));
    ctx.add_rigid_body(
        ball,
        RigidBodyBuilder::dynamic().translation(Vec3::new(0.0, 1.0, 0.0)),
        vec![ColliderBuilder::ball(0.5)],
    )
    .unwrap();
    world.insert_resource(ctx);

    tick_physics(&mut world, DT);
}
