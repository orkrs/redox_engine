//! Specification (tests first) for `stay` contact events (plan item B3, remaining part).
//!
//! `ContactEvent` has only `Enter` and `Exit` and several tests match it exhaustively, so `stay` is a
//! separate, opt-in stream. API to implement in `redox_physics` (in `contacts.rs`, re-exported from the
//! crate root):
//!
//! ```text
//! #[derive(Clone, Copy, Debug, PartialEq)]
//! pub struct ContactStay { pub a: Entity, pub b: Entity, pub impulse: f32 }   // impulse: total normal impulse this step
//! impl PhysicsContext {
//!     /// Opt in or out of per-step `stay` reports for the pairs the entity is part of. Default: off.
//!     /// Independent of `set_contact_events`.
//!     pub fn set_contact_stay(&mut self, entity: Entity, enabled: bool) -> Result<(), PhysicsError>;
//!     /// Takes the stay reports collected since the last call (oldest first).
//!     pub fn drain_contact_stays(&mut self) -> Vec<ContactStay>;
//! }
//! ```
//!
//! A pair produces one `ContactStay` per `step` while its colliders are touching with at least one active
//! contact point, if at least one of the two entities opted in. The step in which a contact begins is the
//! `Enter` step; a `Stay` is reported for it as well only if the contact is still touching at the end of the
//! step (it is). `tick_physics` forwards stays into `Events<ContactStay>` when that resource exists.

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::{Entity, Events, World};
use redox_math::Vec3;
use redox_physics::{ContactEvent, ContactStay, PhysicsContext, sync::tick_physics};

const DT: f32 = 1.0 / 60.0;

fn resting_ball() -> (PhysicsContext, Entity, Entity) {
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
        RigidBodyBuilder::dynamic().translation(Vec3::new(0.0, 0.52, 0.0)),
        vec![ColliderBuilder::ball(0.5)],
    )
    .unwrap();
    (ctx, floor, ball)
}

#[test]
fn a_resting_ball_reports_a_stay_every_step() {
    let (mut ctx, floor, ball) = resting_ball();
    ctx.set_contact_stay(ball, true).unwrap();
    for _ in 0..30 {
        ctx.step(DT); // settle
    }
    ctx.drain_contact_stays();
    for _ in 0..30 {
        ctx.step(DT);
    }
    let stays = ctx.drain_contact_stays();
    assert!(stays.len() >= 28, "about one per step: {}", stays.len());
    for stay in &stays {
        assert!(
            (stay.a == floor && stay.b == ball) || (stay.a == ball && stay.b == floor),
            "{stay:?}"
        );
        assert!(stay.impulse > 0.0, "the floor pushes up: {stay:?}");
    }
    assert!(
        ctx.drain_contact_stays().is_empty(),
        "drain empties the queue"
    );
}

#[test]
fn the_resting_impulse_balances_gravity() {
    // 1 kg ball: weight impulse per step = m * g * dt = 0.1635.
    let (mut ctx, _, ball) = resting_ball();
    ctx.set_contact_stay(ball, true).unwrap();
    for _ in 0..60 {
        ctx.step(DT);
    }
    ctx.drain_contact_stays();
    ctx.step(DT);
    let stay = ctx.drain_contact_stays();
    assert_eq!(stay.len(), 1);
    let mass = {
        // density 1 ball of radius 0.5
        4.0 / 3.0 * std::f32::consts::PI * 0.5f32.powi(3)
    };
    let expected = mass * 9.81 * DT;
    assert!(
        (stay[0].impulse - expected).abs() < expected * 0.35,
        "impulse {} vs weight impulse {expected}",
        stay[0].impulse
    );
}

#[test]
fn stays_are_opt_in_and_either_entity_is_enough() {
    let (mut ctx, floor, _) = resting_ball();
    for _ in 0..20 {
        ctx.step(DT);
    }
    assert!(ctx.drain_contact_stays().is_empty(), "off by default");
    ctx.set_contact_stay(floor, true).unwrap();
    for _ in 0..10 {
        ctx.step(DT);
    }
    assert!(!ctx.drain_contact_stays().is_empty());
    ctx.set_contact_stay(floor, false).unwrap();
    for _ in 0..10 {
        ctx.step(DT);
    }
    assert!(ctx.drain_contact_stays().is_empty(), "off again");
}

#[test]
fn stays_do_not_disturb_enter_and_exit_events() {
    let (mut ctx, _, ball) = resting_ball();
    ctx.set_contact_events(ball, true).unwrap();
    ctx.set_contact_stay(ball, true).unwrap();
    for _ in 0..60 {
        ctx.step(DT);
    }
    let events = ctx.drain_contact_events();
    let enters = events
        .iter()
        .filter(|e| matches!(e, ContactEvent::Enter(_)))
        .count();
    assert!(
        enters <= 1,
        "no Enter spam from the stay reporting: {events:?}"
    );
}

#[test]
fn a_ball_in_the_air_has_no_stays() {
    let mut world = World::new();
    let ball = world.spawn();
    let mut ctx = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));
    ctx.add_rigid_body(
        ball,
        RigidBodyBuilder::dynamic().translation(Vec3::new(0.0, 100.0, 0.0)),
        vec![ColliderBuilder::ball(0.5)],
    )
    .unwrap();
    ctx.set_contact_stay(ball, true).unwrap();
    for _ in 0..30 {
        ctx.step(DT);
    }
    assert!(ctx.drain_contact_stays().is_empty());
}

#[test]
fn unknown_entities_are_an_error() {
    let (mut ctx, _, _) = resting_ball();
    let mut other = World::new();
    let stranger = (0..8).map(|_| other.spawn()).last().unwrap();
    assert!(ctx.set_contact_stay(stranger, true).is_err());
}

#[test]
fn tick_physics_forwards_stays_into_the_ecs() {
    let (mut ctx, _, ball) = resting_ball();
    ctx.set_contact_stay(ball, true).unwrap();
    let mut world = World::new();
    world.insert_resource(ctx);
    world.insert_resource(Events::<ContactStay>::new());
    for _ in 0..30 {
        tick_physics(&mut world, DT);
    }
    let events = world.get_resource::<Events<ContactStay>>().unwrap();
    assert!(events.iter().count() >= 20);
}
