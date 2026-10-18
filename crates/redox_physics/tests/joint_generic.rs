//! Specification (tests first) for generic joints (plan item B4, last part).
//!
//! Add to `joints.rs` (re-export `JointAxes` from `lib.rs` next to `JointDesc`):
//!
//! ```text
//! /// Which of the six relative degrees of freedom a generic joint LOCKS.
//! #[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
//! pub struct JointAxes {
//!     pub lin_x: bool, pub lin_y: bool, pub lin_z: bool,
//!     pub ang_x: bool, pub ang_y: bool, pub ang_z: bool,
//! }
//! impl JointAxes {
//!     pub const NONE: JointAxes;     // nothing locked
//!     pub const ALL: JointAxes;      // everything locked (behaves like Fixed)
//!     pub const LINEAR: JointAxes;   // the three linear axes locked (behaves like Ball)
//! }
//! JointKind::Generic { locked: JointAxes }     // new variant, axes in the local frame of body A
//! ```
//!
//! `JointDesc::limits` and `motor` are ignored for `Generic` (they keep their meaning for Hinge and
//! Slider). The variant is `Copy`. In rapier: `GenericJointBuilder::new(JointAxesMask)` where the mask is
//! built from `JointAxesMask::LIN_X | ...`; then set the anchors as `to_generic` already does.
//! `JointKind` is matched in `to_generic` only; keep every existing test passing.

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::{Entity, World};
use redox_math::Vec3;
use redox_physics::{JointAxes, JointDesc, JointKind, PhysicsContext};

const DT: f32 = 1.0 / 60.0;

fn position(ctx: &PhysicsContext, e: Entity) -> Vec3 {
    ctx.rigid_bodies[ctx.get_body_handle(e).unwrap()].translation()
}

/// A fixed anchor and a 1 kg body at the origin, joined with `locked`; returns where the body is after
/// 2 s and how fast it spins.
fn run(locked: JointAxes, gravity: Vec3) -> (Vec3, f32) {
    let mut world = World::new();
    let anchor = world.spawn();
    let body = world.spawn();
    let mut ctx = PhysicsContext::new(gravity);
    ctx.add_rigid_body(
        anchor,
        RigidBodyBuilder::fixed(),
        vec![ColliderBuilder::ball(0.1)],
    )
    .unwrap();
    ctx.add_rigid_body(
        body,
        RigidBodyBuilder::dynamic(),
        vec![ColliderBuilder::cuboid(0.3, 0.3, 0.3).mass(1.0)],
    )
    .unwrap();
    ctx.add_joint(anchor, body, JointDesc::new(JointKind::Generic { locked }))
        .unwrap();
    for _ in 0..120 {
        ctx.step(DT);
    }
    let handle = ctx.get_body_handle(body).unwrap();
    (
        position(&ctx, body),
        ctx.rigid_bodies[handle].angvel().length(),
    )
}

#[test]
fn the_presets_describe_the_expected_locks() {
    assert!(!JointAxes::NONE.lin_x && !JointAxes::NONE.ang_z);
    assert!(JointAxes::ALL.lin_x && JointAxes::ALL.lin_y && JointAxes::ALL.lin_z);
    assert!(JointAxes::ALL.ang_x && JointAxes::ALL.ang_y && JointAxes::ALL.ang_z);
    assert!(JointAxes::LINEAR.lin_x && JointAxes::LINEAR.lin_y && JointAxes::LINEAR.lin_z);
    assert!(!JointAxes::LINEAR.ang_x && !JointAxes::LINEAR.ang_y && !JointAxes::LINEAR.ang_z);
    assert_eq!(JointAxes::default(), JointAxes::NONE);
}

#[test]
fn locking_everything_holds_the_body_against_gravity() {
    let (p, _) = run(JointAxes::ALL, Vec3::new(0.0, -9.81, 0.0));
    assert!(p.length() < 0.02, "{p:?}");
}

#[test]
fn nothing_locked_lets_the_body_fall_freely() {
    let (p, _) = run(JointAxes::NONE, Vec3::new(0.0, -9.81, 0.0));
    // 2 s of free fall is about 19.6 m.
    assert!(p.y < -15.0, "{p:?}");
}

#[test]
fn a_single_free_linear_axis_is_a_slider() {
    // Everything locked except motion along Y: gravity along -Y drops the body, gravity along -X does not.
    let mut slide_y = JointAxes::ALL;
    slide_y.lin_y = false;
    let (fall, _) = run(slide_y, Vec3::new(0.0, -9.81, 0.0));
    assert!(fall.y < -15.0, "{fall:?}");
    assert!(fall.x.abs() < 0.02 && fall.z.abs() < 0.02, "{fall:?}");
    let (sideways, _) = run(slide_y, Vec3::new(-9.81, 0.0, 0.0));
    assert!(sideways.length() < 0.02, "{sideways:?}");
}

#[test]
fn locked_linear_axes_with_free_rotation_act_like_a_ball_joint() {
    let (p, _) = run(JointAxes::LINEAR, Vec3::new(0.0, -9.81, 0.0));
    assert!(p.length() < 0.02, "the body stays at the anchor: {p:?}");
}

#[test]
fn a_fully_locked_joint_does_not_spin() {
    let (_, spin) = run(JointAxes::ALL, Vec3::ZERO);
    assert!(spin < 1e-3);
}
