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

/// One degree in radians.
const DEG: f32 = std::f32::consts::PI / 180.0;

/// Result of a ball-joint run: the joint id and the largest angle between the rod and straight down.
struct BallRun {
    ctx: PhysicsContext,
    joint: redox_physics::JointId,
    max_angle: f32,
}

/// A 1 kg bob 1 m below a fixed pivot at the origin, joined by `kind` under `gravity`.
fn run_ball_pendulum(
    kind: JointKind,
    gravity: Vec3,
    break_force: Option<f32>,
    seconds: f32,
) -> BallRun {
    let mut world = World::new();
    let pivot = world.spawn();
    let bob = world.spawn();
    let mut ctx = PhysicsContext::new(gravity);
    ctx.add_rigid_body(
        pivot,
        RigidBodyBuilder::fixed(),
        vec![ColliderBuilder::ball(0.05)],
    )
    .unwrap();
    ctx.add_rigid_body(
        bob,
        RigidBodyBuilder::dynamic().translation(Vec3::new(0.0, -1.0, 0.0)),
        vec![ColliderBuilder::ball(0.05).mass(1.0)],
    )
    .unwrap();
    let mut desc = JointDesc::new(kind);
    desc.anchor_b = Vec3::new(0.0, 1.0, 0.0);
    desc.break_force = break_force;
    let joint = ctx.add_joint(pivot, bob, desc).unwrap();
    let steps = (seconds / DT).round() as usize;
    let mut max_angle = 0.0_f32;
    for _ in 0..steps {
        ctx.step(DT);
        let p = position(&ctx, bob);
        max_angle = max_angle.max((-p.y / p.length()).clamp(-1.0, 1.0).acos());
    }
    BallRun {
        ctx,
        joint,
        max_angle,
    }
}

#[test]
fn a_limited_ball_cannot_swing_beyond_its_cone() {
    let kind = JointKind::BallLimited {
        swing_limit: 45.0 * DEG,
        twist_min: -std::f32::consts::FRAC_PI_2,
        twist_max: std::f32::consts::FRAC_PI_2,
    };
    // Sideways gravity pushes the bob against the 45 degree stop.
    let run = run_ball_pendulum(kind, Vec3::new(9.81, 0.0, 0.0), None, 3.0);
    assert!(
        run.max_angle <= 47.0 * DEG,
        "swing {} deg exceeds the cone",
        run.max_angle / DEG
    );
    assert!(run.max_angle > 40.0 * DEG, "did reach the cone edge");
}

#[test]
fn the_twist_is_limited() {
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
        RigidBodyBuilder::dynamic(),
        vec![ColliderBuilder::ball(0.05).mass(1.0)],
    )
    .unwrap();
    let kind = JointKind::BallLimited {
        swing_limit: std::f32::consts::FRAC_PI_2,
        twist_min: -0.3,
        twist_max: 0.3,
    };
    ctx.add_joint(pivot, bob, JointDesc::new(kind)).unwrap();
    let handle = ctx.get_body_handle(bob).unwrap();
    // A push about the joint's X axis (the twist axis) through the shared centre.
    ctx.set_rigid_body_velocity(handle, Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0));
    let (mut lowest, mut highest) = (f32::MAX, f32::MIN);
    for _ in 0..60 {
        ctx.step(DT);
        let (axis, angle) = ctx
            .get_rigid_body_transform(handle)
            .unwrap()
            .1
            .to_axis_angle();
        let twist = angle * axis.x;
        lowest = lowest.min(twist);
        highest = highest.max(twist);
    }
    assert!(highest > 0.25, "the twist limit was reached: {highest}");
    assert!(highest <= 0.3 + 2.0 * DEG, "twist above max: {highest}");
    assert!(lowest >= -0.3 - 2.0 * DEG, "twist below min: {lowest}");
}

#[test]
fn an_unlimited_ball_still_swings_freely() {
    let run = run_ball_pendulum(JointKind::Ball, Vec3::new(9.81, 0.0, 0.0), None, 3.0);
    assert!(
        run.max_angle > 90.0 * DEG,
        "free swing: {} deg",
        run.max_angle / DEG
    );
}

#[test]
fn the_limits_hold_while_the_joint_is_stressed_but_not_broken() {
    let kind = JointKind::BallLimited {
        swing_limit: 45.0 * DEG,
        twist_min: -std::f32::consts::FRAC_PI_2,
        twist_max: std::f32::consts::FRAC_PI_2,
    };
    // Ten times gravity, with a break force far above the load.
    let run = run_ball_pendulum(kind, Vec3::new(98.1, 0.0, 0.0), Some(1.0e6), 3.0);
    assert!(run.ctx.joint_exists(run.joint), "the joint is not broken");
    assert!(
        run.max_angle <= 47.0 * DEG,
        "limit still holds: {} deg",
        run.max_angle / DEG
    );
}
