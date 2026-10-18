//! Specification (tests first) for a contact impulse threshold (plan item B3, "thresholds for sound").
//!
//! A ball resting on a floor or rolling gently produces `Enter` events with a tiny impulse; a sound
//! system wants only the hard hits. API to add to `PhysicsContext` (in `contacts.rs`):
//!
//! ```text
//! /// Contact `Enter` events whose impulse is below `min_impulse` are dropped, and so is the `Exit` of
//! /// a pair whose `Enter` was dropped. Default 0.0 (nothing is dropped). Negative and NaN values are
//! /// treated as 0.0. Applies to events produced after the call.
//! pub fn set_contact_impulse_threshold(&mut self, min_impulse: f32);
//! /// The current threshold.
//! pub fn contact_impulse_threshold(&self) -> f32;
//! ```
//!
//! Hint: keep a `HashSet` of suppressed pairs (entity pair stored in a normalised order) in the context;
//! `collect_contact_events` inserts a pair when it drops an `Enter` and removes it when the matching
//! `Exit` arrives. Stay reports (`ContactStay`) are not affected.

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::World;
use redox_math::Vec3;
use redox_physics::{ContactEvent, PhysicsContext};

const DT: f32 = 1.0 / 60.0;

/// Drops a ball from `height` metres above a floor; returns the context after the landing and the
/// events of the whole run.
fn drop_ball(threshold: f32, height: f32) -> Vec<ContactEvent> {
    let mut world = World::new();
    let floor = world.spawn();
    let ball = world.spawn();
    let mut ctx = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));
    ctx.set_contact_impulse_threshold(threshold);
    ctx.add_rigid_body(
        floor,
        RigidBodyBuilder::fixed().translation(Vec3::new(0.0, -0.5, 0.0)),
        vec![ColliderBuilder::cuboid(50.0, 0.5, 50.0)],
    )
    .unwrap();
    ctx.add_rigid_body(
        ball,
        RigidBodyBuilder::dynamic().translation(Vec3::new(0.0, 0.5 + height, 0.0)),
        vec![ColliderBuilder::ball(0.5).restitution(0.0)],
    )
    .unwrap();
    ctx.set_contact_events(ball, true).unwrap();
    for _ in 0..180 {
        ctx.step(DT);
    }
    ctx.drain_contact_events()
}

fn enters(events: &[ContactEvent]) -> Vec<f32> {
    events
        .iter()
        .filter_map(|e| match e {
            ContactEvent::Enter(enter) => Some(enter.impulse),
            ContactEvent::Exit(_) => None,
        })
        .collect()
}

fn exit_count(events: &[ContactEvent]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, ContactEvent::Exit(_)))
        .count()
}

#[test]
fn the_default_threshold_is_zero_and_drops_nothing() {
    let ctx = PhysicsContext::new(Vec3::ZERO);
    assert_eq!(ctx.contact_impulse_threshold(), 0.0);
    assert!(!enters(&drop_ball(0.0, 0.05)).is_empty());
}

#[test]
fn bad_thresholds_are_treated_as_zero() {
    let mut ctx = PhysicsContext::new(Vec3::ZERO);
    ctx.set_contact_impulse_threshold(-3.0);
    assert_eq!(ctx.contact_impulse_threshold(), 0.0);
    ctx.set_contact_impulse_threshold(f32::NAN);
    assert_eq!(ctx.contact_impulse_threshold(), 0.0);
    ctx.set_contact_impulse_threshold(2.5);
    assert_eq!(ctx.contact_impulse_threshold(), 2.5);
}

#[test]
fn a_threshold_between_a_soft_and_a_hard_hit_keeps_only_the_hard_one() {
    let soft = enters(&drop_ball(0.0, 0.02));
    let hard = enters(&drop_ball(0.0, 3.0));
    let (soft, hard) = (soft[0], hard[0]);
    assert!(hard > soft * 2.0, "setup: soft {soft}, hard {hard}");
    let between = (soft + hard) * 0.5;

    let kept_soft = drop_ball(between, 0.02);
    assert!(enters(&kept_soft).is_empty(), "soft hit is dropped");
    assert_eq!(exit_count(&kept_soft), 0, "and so is its exit");

    let kept_hard = drop_ball(between, 3.0);
    assert_eq!(enters(&kept_hard).len(), 1, "hard hit is kept");
}
