//! The break force is a real force in newtons: a joint holding a 1 kg body (9.81 N) breaks with a
//! limit below that and holds with a limit above it. (rapier reports the impulse of one solver
//! substep, so reading it as a per-step impulse understated the force several times over.)

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::World;
use redox_math::Vec3;
use redox_physics::{JointDesc, JointKind, PhysicsContext};

fn holds(break_force: f32) -> bool {
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
        RigidBodyBuilder::dynamic().translation(Vec3::new(0.0, 9.0, 0.0)),
        vec![ColliderBuilder::ball(0.1).mass(1.0)],
    )
    .unwrap();
    let mut desc = JointDesc::new(JointKind::Fixed);
    desc.anchor_a = Vec3::new(0.0, -1.0, 0.0);
    desc.break_force = Some(break_force);
    let joint = ctx.add_joint(pivot, bob, desc).unwrap();
    for _ in 0..120 {
        ctx.step(1.0 / 60.0);
    }
    ctx.joint_exists(joint)
}

#[test]
fn the_break_force_is_measured_in_newtons() {
    assert!(!holds(8.0), "9.81 N of weight exceeds an 8 N limit");
    assert!(holds(12.0), "9.81 N of weight stays under a 12 N limit");
}
