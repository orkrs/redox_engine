//! Specification (tests first) for joint limits (plan item B4, remaining part).
//!
//! API to add in `joints.rs` (re-exports in `lib.rs` need no change for this part):
//!
//! ```text
//! JointDesc { ..., pub limits: Option<(f32, f32)> }
//! ```
//!
//! `limits` is `(min, max)`: radians of rotation for `JointKind::Hinge`, metres of travel for
//! `JointKind::Slider`, measured from the pose in which the joint was created. It is ignored for the
//! other kinds. `JointDesc::new` sets `None`. `min > max` or NaN are treated as no limits. The struct
//! stays `Copy`, and the existing joint tests must pass unchanged.
//!
//! In rapier: `RevoluteJointBuilder::limits([min, max])`, `PrismaticJointBuilder::limits([min, max])`.

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::World;
use redox_math::Vec3;
use redox_physics::{JointDesc, JointKind, PhysicsContext};

const DT: f32 = 1.0 / 60.0;

fn position(ctx: &PhysicsContext, entity: redox_ecs::Entity) -> Vec3 {
    let handle = ctx.get_body_handle(entity).unwrap();
    ctx.rigid_bodies[handle].translation()
}

/// A 1 kg bob 1 m to the side of a fixed pivot at the origin, hinged about Z; gravity swings it down.
/// Returns the lowest y the bob reached in 4 s.
fn lowest_swing(limits: Option<(f32, f32)>) -> f32 {
    let mut world = World::new();
    let pivot = world.spawn();
    let bob = world.spawn();
    let mut ctx = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));
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
    desc.limits = limits;
    ctx.add_joint(pivot, bob, desc).unwrap();
    let mut lowest = f32::MAX;
    for _ in 0..240 {
        ctx.step(DT);
        lowest = lowest.min(position(&ctx, bob).y);
    }
    lowest
}

#[test]
fn a_hinge_without_limits_swings_down_to_the_bottom() {
    assert!(lowest_swing(None) < -0.95);
}

#[test]
fn a_hinge_limit_stops_the_swing() {
    // 0.5 rad: the bob hangs at y = -sin(0.5) = -0.479 at most.
    let lowest = lowest_swing(Some((-0.5, 0.5)));
    assert!(lowest > -0.56, "stopped by the limit: {lowest}");
    assert!(lowest < -0.4, "but it did swing down to it: {lowest}");
}

#[test]
fn reversed_or_nan_limits_mean_no_limits() {
    assert!(lowest_swing(Some((0.5, -0.5))) < -0.95);
    assert!(lowest_swing(Some((f32::NAN, 0.5))) < -0.95);
}

#[test]
fn a_slider_limit_stops_the_fall() {
    let mut world = World::new();
    let rail = world.spawn();
    let car = world.spawn();
    let mut ctx = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));
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
    let mut desc = JointDesc::new(JointKind::Slider { axis: Vec3::Y });
    desc.limits = Some((-0.5, 0.5));
    ctx.add_joint(rail, car, desc).unwrap();
    for _ in 0..180 {
        ctx.step(DT);
    }
    let y = position(&ctx, car).y;
    assert!((-0.56..-0.44).contains(&y), "rests on the lower stop: {y}");
}
