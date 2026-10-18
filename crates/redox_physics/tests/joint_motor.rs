//! Specification (tests first) for joint motors (plan item B4, remaining part).
//!
//! API to add in `joints.rs`; `JointMotor` is re-exported from the crate root next to `JointDesc`:
//!
//! ```text
//! #[derive(Clone, Copy, Debug, PartialEq)]
//! pub struct JointMotor {
//!     /// Target speed: rad/s for a hinge, m/s for a slider.
//!     pub target_velocity: f32,
//!     /// Largest force the motor may apply: N*m for a hinge, N for a slider.
//!     pub max_force: f32,
//! }
//! JointDesc { ..., pub motor: Option<JointMotor> }   // `JointDesc::new` sets None; ignored for other kinds
//! ```
//!
//! A motor drives the joint towards `target_velocity` without exceeding `max_force`. In rapier:
//! `RevoluteJointBuilder::motor_velocity(target, factor)` + `.motor_max_force(max)`, the same names on
//! `PrismaticJointBuilder`. Use a stiff `factor` (for example 1e3) so the target is reached quickly. Non
//! finite values (NaN, infinity) in `JointMotor` mean no motor. `JointDesc` stays `Copy`.

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::World;
use redox_math::Vec3;
use redox_physics::{JointDesc, JointKind, JointMotor, PhysicsContext};

const DT: f32 = 1.0 / 60.0;

fn position(ctx: &PhysicsContext, entity: redox_ecs::Entity) -> Vec3 {
    let handle = ctx.get_body_handle(entity).unwrap();
    ctx.rigid_bodies[handle].translation()
}

/// Weightless 1 kg bob 1 m from a fixed pivot, hinged about Z with the given motor; returns the angle
/// (radians, absolute) the bob turned through after 1 s.
fn turned(motor: Option<JointMotor>) -> f32 {
    let mut world = World::new();
    let pivot = world.spawn();
    let bob = world.spawn();
    let mut ctx = PhysicsContext::new(Vec3::ZERO);
    ctx.add_rigid_body(
        pivot,
        RigidBodyBuilder::fixed(),
        vec![ColliderBuilder::ball(0.05)],
    )
    .unwrap();
    ctx.add_rigid_body(
        bob,
        RigidBodyBuilder::dynamic().translation(Vec3::new(1.0, 0.0, 0.0)),
        vec![ColliderBuilder::ball(0.05).mass(1.0)],
    )
    .unwrap();
    let mut desc = JointDesc::new(JointKind::Hinge { axis: Vec3::Z });
    desc.anchor_b = Vec3::new(-1.0, 0.0, 0.0);
    desc.motor = motor;
    ctx.add_joint(pivot, bob, desc).unwrap();
    for _ in 0..60 {
        ctx.step(DT);
    }
    let p = position(&ctx, bob);
    p.y.atan2(p.x).abs()
}

#[test]
fn without_a_motor_a_weightless_hinge_stays_put() {
    assert!(turned(None) < 0.01);
}

#[test]
fn a_strong_motor_turns_the_hinge_at_the_target_speed() {
    let motor = JointMotor {
        target_velocity: 2.0,
        max_force: 1000.0,
    };
    let angle = turned(Some(motor));
    assert!((1.7..2.1).contains(&angle), "about 2 rad in 1 s: {angle}");
}

#[test]
fn a_weak_motor_is_limited_by_its_max_force() {
    let motor = JointMotor {
        target_velocity: 2.0,
        max_force: 0.05,
    };
    // 0.05 N*m on a 1 kg bob at 1 m: angular acceleration ~0.05 rad/s^2, so ~0.025 rad in 1 s.
    let angle = turned(Some(motor));
    assert!(angle < 0.1, "force limited: {angle}");
}

#[test]
fn non_finite_motors_are_ignored() {
    let motor = JointMotor {
        target_velocity: f32::NAN,
        max_force: 1000.0,
    };
    assert!(turned(Some(motor)) < 0.01);
}

#[test]
fn a_slider_motor_pushes_along_the_axis() {
    let mut world = World::new();
    let rail = world.spawn();
    let car = world.spawn();
    let mut ctx = PhysicsContext::new(Vec3::ZERO);
    ctx.add_rigid_body(
        rail,
        RigidBodyBuilder::fixed(),
        vec![ColliderBuilder::ball(0.05)],
    )
    .unwrap();
    ctx.add_rigid_body(
        car,
        RigidBodyBuilder::dynamic(),
        vec![ColliderBuilder::ball(0.05).mass(1.0)],
    )
    .unwrap();
    let mut desc = JointDesc::new(JointKind::Slider { axis: Vec3::X });
    desc.motor = Some(JointMotor {
        target_velocity: 1.5,
        max_force: 1000.0,
    });
    ctx.add_joint(rail, car, desc).unwrap();
    for _ in 0..60 {
        ctx.step(DT);
    }
    let x = position(&ctx, car).x.abs();
    assert!((1.2..1.6).contains(&x), "about 1.5 m in 1 s: {x}");
}
