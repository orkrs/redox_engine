//! Specification (tests first) for breaking joints by torque (plan item B4, remaining part).
//!
//! The break check in `joints.rs` only looks at the linear part of the joint impulse. Add:
//!
//! ```text
//! JointDesc { ..., pub break_torque: Option<f32> }   // N*m; `JointDesc::new` sets it to None
//! ```
//!
//! After each `step` a joint is also removed (and reported once through `drain_joint_broken`) when the
//! magnitude of its ANGULAR constraint impulse divided by `dt` exceeds `break_torque`. The linear check
//! with `break_force` stays as it is; either limit breaks the joint. `JointDesc` stays `Copy` and the
//! existing `tests/joints.rs` must pass unchanged (it builds descriptions with `JointDesc::new` and field
//! assignment, so a new field with a `None` default is compatible).
//!
//! Scenario: a fixed joint holds a 1 kg ball whose centre is 2 m to the side of the joint anchor. The
//! weight (9.81 N) is carried by the force part (about 9.81 N) and the torque part (about 19.6 N*m).

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::World;
use redox_math::Vec3;
use redox_physics::{JointDesc, JointKind, PhysicsContext};

const DT: f32 = 1.0 / 60.0;

/// Pivot at (0, 10, 0); a ball 2 m to the +x side held by a fixed joint with the given limits.
fn lever(break_force: Option<f32>, break_torque: Option<f32>) -> (PhysicsContext, bool) {
    let mut world = World::new();
    let pivot = world.spawn();
    let bob = world.spawn();
    let mut ctx = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));
    ctx.add_rigid_body(
        pivot,
        RigidBodyBuilder::fixed().translation(Vec3::new(0.0, 10.0, 0.0)),
        vec![ColliderBuilder::ball(0.1)],
    )
    .unwrap();
    ctx.add_rigid_body(
        bob,
        RigidBodyBuilder::dynamic().translation(Vec3::new(2.0, 10.0, 0.0)),
        vec![ColliderBuilder::ball(0.1).mass(1.0)],
    )
    .unwrap();
    let mut desc = JointDesc::new(JointKind::Fixed);
    desc.anchor_b = Vec3::new(-2.0, 0.0, 0.0);
    desc.break_force = break_force;
    desc.break_torque = break_torque;
    let joint = ctx.add_joint(pivot, bob, desc).unwrap();
    for _ in 0..120 {
        ctx.step(DT);
    }
    let broke = !ctx.joint_exists(joint);
    (ctx, broke)
}

#[test]
fn a_low_torque_limit_breaks_a_joint_the_force_limit_alone_would_keep() {
    let (mut ctx, broke) = lever(Some(1000.0), Some(5.0));
    assert!(broke, "19.6 N*m exceeds 5 N*m");
    assert_eq!(ctx.drain_joint_broken().len(), 1);
    assert!(ctx.drain_joint_broken().is_empty(), "reported once");
}

#[test]
fn a_high_torque_limit_holds() {
    let (mut ctx, broke) = lever(Some(1000.0), Some(1000.0));
    assert!(!broke);
    assert!(ctx.drain_joint_broken().is_empty());
}

#[test]
fn no_torque_limit_means_torque_never_breaks_the_joint() {
    let (_, broke) = lever(Some(1000.0), None);
    assert!(!broke);
    let (_, broke) = lever(None, None);
    assert!(!broke);
}

#[test]
fn the_force_limit_still_works_on_its_own() {
    let (_, broke) = lever(Some(2.0), None);
    assert!(broke, "9.81 N exceeds 2 N");
}

#[test]
fn the_default_description_has_no_torque_limit() {
    assert_eq!(JointDesc::new(JointKind::Ball).break_torque, None);
}
