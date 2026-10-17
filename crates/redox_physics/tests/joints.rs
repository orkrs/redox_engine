//! Specification (tests first) for joints (plan item B4).
//!
//! API to implement in `redox_physics` (new module `joints.rs`, re-exported from the crate root):
//!
//! ```text
//! #[derive(Clone, Copy, Debug, PartialEq)]
//! pub enum JointKind {
//!     Fixed,                                                       // no relative motion
//!     Hinge { axis: Vec3 },                                        // rotation about `axis` only
//!     Slider { axis: Vec3 },                                       // translation along `axis` only
//!     Ball,                                                        // free rotation about the anchor
//!     Spring { rest_length: f32, stiffness: f32, damping: f32 },   // force based: F = k * extension (N/m)
//!     Rope { max_length: f32 },                                    // anchors may not be farther apart
//! }
//! #[derive(Clone, Copy, Debug, PartialEq)]
//! pub struct JointDesc {
//!     pub kind: JointKind,
//!     pub anchor_a: Vec3,            // in the local space of body A
//!     pub anchor_b: Vec3,            // in the local space of body B
//!     pub break_force: Option<f32>,  // the joint is removed when its constraint force exceeds this (N)
//! }
//! impl JointDesc { pub fn new(kind: JointKind) -> Self }           // anchors zero, no break force
//! #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct JointId(/* opaque */);
//! #[derive(Clone, Copy, Debug, PartialEq)]
//! pub struct JointBroken { pub joint: JointId, pub a: Entity, pub b: Entity }
//! impl PhysicsContext {
//!     pub fn add_joint(&mut self, a: Entity, b: Entity, desc: JointDesc) -> Result<JointId, PhysicsError>;
//!     pub fn remove_joint(&mut self, joint: JointId) -> bool;       // false when it did not exist
//!     pub fn joint_exists(&self, joint: JointId) -> bool;
//!     /// Takes the joints that broke since the last call (oldest first).
//!     pub fn drain_joint_broken(&mut self) -> Vec<JointBroken>;
//! }
//! ```
//!
//! Notes: bodies joined by a joint do not collide with each other. A body removed with
//! `remove_rigid_body` takes its joints with it. `add_joint` fails with
//! `PhysicsError::EntityNotFound` when either entity has no body. A joint that breaks is removed and
//! reported once. Break force is checked every `step`.

#![allow(clippy::unwrap_used)]

use rapier3d::dynamics::RigidBodyHandle;
use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::{Entity, World};
use redox_math::Vec3;
use redox_physics::{JointDesc, JointKind, PhysicsContext};

const DT: f32 = 1.0 / 60.0;
const PIVOT: Vec3 = Vec3::new(0.0, 10.0, 0.0);

struct Rig {
    ctx: PhysicsContext,
    pivot: Entity,
    bob: Entity,
    bob_handle: RigidBodyHandle,
}

/// A fixed pivot at (0, 10, 0) and a 1 kg ball at `bob_at`, gravity 9.81 downwards.
fn rig(bob_at: Vec3, velocity: Vec3) -> Rig {
    let mut world = World::new();
    let pivot = world.spawn();
    let bob = world.spawn();
    let mut ctx = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));
    ctx.add_rigid_body(
        pivot,
        RigidBodyBuilder::fixed().translation(PIVOT),
        vec![ColliderBuilder::ball(0.1)],
    )
    .unwrap();
    let (bob_handle, _) = ctx
        .add_rigid_body(
            bob,
            RigidBodyBuilder::dynamic()
                .translation(bob_at)
                .linvel(velocity),
            vec![ColliderBuilder::ball(0.1).mass(1.0)],
        )
        .unwrap();
    Rig {
        ctx,
        pivot,
        bob,
        bob_handle,
    }
}

impl Rig {
    fn run(&mut self, steps: usize) {
        for _ in 0..steps {
            self.ctx.step(DT);
        }
    }

    fn bob(&self) -> Vec3 {
        self.ctx
            .get_rigid_body_transform(self.bob_handle)
            .unwrap()
            .0
    }

    fn join(&mut self, desc: JointDesc) -> redox_physics::JointId {
        self.ctx.add_joint(self.pivot, self.bob, desc).unwrap()
    }
}

#[test]
fn a_fixed_joint_holds_the_body_against_gravity() {
    let mut rig = rig(PIVOT + Vec3::new(0.0, -1.0, 0.0), Vec3::ZERO);
    let mut desc = JointDesc::new(JointKind::Fixed);
    desc.anchor_a = Vec3::new(0.0, -1.0, 0.0);
    rig.join(desc);
    rig.run(120);
    let p = rig.bob();
    assert!((p - Vec3::new(0.0, 9.0, 0.0)).length() < 0.05, "{p:?}");
}

#[test]
fn a_hinge_swings_like_a_pendulum_in_its_plane() {
    let mut rig = rig(PIVOT + Vec3::new(1.0, 0.0, 0.0), Vec3::ZERO);
    let mut desc = JointDesc::new(JointKind::Hinge { axis: Vec3::Z });
    desc.anchor_b = Vec3::new(-1.0, 0.0, 0.0);
    rig.join(desc);
    let mut lowest = f32::MAX;
    for _ in 0..90 {
        rig.ctx.step(DT);
        let p = rig.bob();
        lowest = lowest.min(p.y);
        assert!(
            ((p - PIVOT).length() - 1.0).abs() < 0.05,
            "stays on the circle: {p:?}"
        );
        assert!(p.z.abs() < 0.05, "stays in the plane: {p:?}");
    }
    assert!(lowest < PIVOT.y - 0.9, "it swings down: lowest {lowest}");
}

#[test]
fn a_slider_moves_only_along_its_axis() {
    let mut rig = rig(PIVOT, Vec3::new(3.0, 0.0, 2.0));
    rig.join(JointDesc::new(JointKind::Slider { axis: Vec3::X }));
    rig.run(60);
    let p = rig.bob();
    assert!((p.x - 3.0).abs() < 0.3, "slides along x: {p:?}");
    assert!((p.y - PIVOT.y).abs() < 0.05, "gravity is held: {p:?}");
    assert!(p.z.abs() < 0.05, "no sideways motion: {p:?}");
}

#[test]
fn a_ball_joint_keeps_the_distance_but_swings_freely() {
    let mut rig = rig(PIVOT + Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 4.0));
    let mut desc = JointDesc::new(JointKind::Ball);
    desc.anchor_b = Vec3::new(-1.0, 0.0, 0.0);
    rig.join(desc);
    let mut max_z = 0.0f32;
    for _ in 0..90 {
        rig.ctx.step(DT);
        let p = rig.bob();
        max_z = max_z.max(p.z.abs());
        assert!(
            ((p - PIVOT).length() - 1.0).abs() < 0.06,
            "constant distance: {p:?}"
        );
    }
    assert!(max_z > 0.5, "it leaves the plane: {max_z}");
}

#[test]
fn a_spring_settles_at_the_extension_gravity_asks_for() {
    // 1 kg, k = 100 N/m: static extension m*g/k = 0.0981 m.
    let mut rig = rig(PIVOT + Vec3::new(0.0, -1.0, 0.0), Vec3::ZERO);
    rig.join(JointDesc::new(JointKind::Spring {
        rest_length: 1.0,
        stiffness: 100.0,
        damping: 8.0,
    }));
    rig.run(600);
    let p = rig.bob();
    assert!(
        (p.y - (PIVOT.y - 1.0 - 0.0981)).abs() < 0.05,
        "settled at {p:?}"
    );
    assert!(p.x.abs() < 0.05 && p.z.abs() < 0.05);
}

#[test]
fn a_rope_is_slack_until_it_is_taut() {
    let mut rig = rig(PIVOT + Vec3::new(0.0, -1.0, 0.0), Vec3::ZERO);
    rig.join(JointDesc::new(JointKind::Rope { max_length: 2.0 }));
    rig.run(12); // 0.2 s of free fall: about 0.2 m, well inside the rope length
    assert!(rig.bob().y < PIVOT.y - 1.1, "free fall while slack");
    for _ in 0..180 {
        rig.ctx.step(DT);
        assert!(
            (rig.bob() - PIVOT).length() < 2.1,
            "never farther than the rope: {:?}",
            rig.bob()
        );
    }
    assert!(rig.bob().y < PIVOT.y - 1.7, "hangs on the taut rope");
}

#[test]
fn a_joint_breaks_when_the_load_exceeds_its_break_force() {
    let mut rig = rig(PIVOT + Vec3::new(0.0, -1.0, 0.0), Vec3::ZERO);
    let mut desc = JointDesc::new(JointKind::Fixed);
    desc.anchor_a = Vec3::new(0.0, -1.0, 0.0);
    desc.break_force = Some(2.0); // the weight alone is 9.81 N
    let joint = rig.join(desc);
    rig.run(90);
    let broken = rig.ctx.drain_joint_broken();
    assert_eq!(broken.len(), 1, "{broken:?}");
    assert_eq!(broken[0].joint, joint);
    assert!(
        (broken[0].a == rig.pivot && broken[0].b == rig.bob)
            || (broken[0].a == rig.bob && broken[0].b == rig.pivot)
    );
    assert!(!rig.ctx.joint_exists(joint));
    assert!(rig.bob().y < 8.0, "it fell: {:?}", rig.bob());
    assert!(rig.ctx.drain_joint_broken().is_empty(), "reported once");
}

#[test]
fn a_strong_joint_does_not_break() {
    let mut rig = rig(PIVOT + Vec3::new(0.0, -1.0, 0.0), Vec3::ZERO);
    let mut desc = JointDesc::new(JointKind::Fixed);
    desc.anchor_a = Vec3::new(0.0, -1.0, 0.0);
    desc.break_force = Some(1000.0);
    let joint = rig.join(desc);
    rig.run(180);
    assert!(rig.ctx.joint_exists(joint));
    assert!(rig.ctx.drain_joint_broken().is_empty());
    assert!((rig.bob().y - 9.0).abs() < 0.05);
}

#[test]
fn removing_a_joint_frees_the_body() {
    let mut rig = rig(PIVOT + Vec3::new(0.0, -1.0, 0.0), Vec3::ZERO);
    let mut desc = JointDesc::new(JointKind::Fixed);
    desc.anchor_a = Vec3::new(0.0, -1.0, 0.0);
    let joint = rig.join(desc);
    rig.run(30);
    assert!(rig.ctx.remove_joint(joint));
    assert!(!rig.ctx.joint_exists(joint));
    assert!(!rig.ctx.remove_joint(joint), "second removal reports false");
    rig.run(60);
    assert!(rig.bob().y < 8.0, "falls once released: {:?}", rig.bob());
}

#[test]
fn removing_a_body_removes_its_joints() {
    let mut rig = rig(PIVOT + Vec3::new(0.0, -1.0, 0.0), Vec3::ZERO);
    let joint = rig.join(JointDesc::new(JointKind::Ball));
    rig.ctx.remove_rigid_body(rig.bob).unwrap();
    assert!(!rig.ctx.joint_exists(joint));
    rig.run(10); // must not panic
}

#[test]
fn joining_unknown_entities_is_an_error() {
    let mut rig = rig(PIVOT, Vec3::ZERO);
    let mut other = World::new();
    let stranger = (0..8).map(|_| other.spawn()).last().unwrap();
    assert!(
        rig.ctx
            .add_joint(rig.pivot, stranger, JointDesc::new(JointKind::Fixed))
            .is_err()
    );
}
