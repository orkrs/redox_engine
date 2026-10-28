//! Tests for the ECS `Joint` component (PHY-04): the component creates, replaces and removes the physics joint.

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::{Entity, Events, World};
use redox_math::{Transform, Vec3};
use redox_physics::{
    Joint, JointBroken, JointDesc, JointKind, JointLink, PhysicsContext, sync::tick_physics,
};

const DT: f32 = 1.0 / 60.0;

fn add_body(world: &mut World, at: Vec3, body: RigidBodyBuilder) -> Entity {
    let e = world.spawn();
    world.add_component(e, Transform::from_translation(at));
    let mut ctx = world.remove_resource::<PhysicsContext>().unwrap();
    ctx.add_rigid_body(
        e,
        body.translation(at),
        vec![ColliderBuilder::ball(0.1).mass(1.0)],
    )
    .unwrap();
    world.insert_resource(ctx);
    e
}

fn two_bodies() -> (World, Entity, Entity) {
    let mut world = World::new();
    world.insert_resource(PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0)));
    let a = add_body(&mut world, Vec3::ZERO, RigidBodyBuilder::fixed());
    let b = add_body(
        &mut world,
        Vec3::new(1.0, 0.0, 0.0),
        RigidBodyBuilder::dynamic(),
    );
    (world, a, b)
}

fn joint_exists(world: &World, link: JointLink) -> bool {
    world
        .get_resource::<PhysicsContext>()
        .unwrap()
        .joint_exists(link.0)
}

#[test]
fn a_joint_component_creates_a_joint_once() {
    let (mut world, a, b) = two_bodies();
    world.add_component(
        a,
        Joint {
            other: b,
            desc: JointDesc::new(JointKind::Fixed),
        },
    );
    tick_physics(&mut world, DT);
    let first = *world.get_component::<JointLink>(a).unwrap();
    assert!(joint_exists(&world, first));
    tick_physics(&mut world, DT);
    let second = *world.get_component::<JointLink>(a).unwrap();
    assert_eq!(first, second, "the joint is not created again");
}

#[test]
fn removing_the_component_removes_the_joint() {
    let (mut world, a, b) = two_bodies();
    world.add_component(
        a,
        Joint {
            other: b,
            desc: JointDesc::new(JointKind::Fixed),
        },
    );
    tick_physics(&mut world, DT);
    let link = *world.get_component::<JointLink>(a).unwrap();
    world.remove_component::<Joint>(a);
    tick_physics(&mut world, DT);
    assert!(world.get_component::<JointLink>(a).is_none());
    assert!(!joint_exists(&world, link));
}

#[test]
fn a_broken_joint_clears_both_components_and_sends_the_event() {
    let (mut world, a, b) = two_bodies();
    world.insert_resource(Events::<JointBroken>::new());
    world.add_component(
        a,
        Joint {
            other: b,
            desc: JointDesc {
                anchor_a: Vec3::new(1.0, 0.0, 0.0), // where b hangs: the weight (9.81 N) alone breaks it
                break_force: Some(2.0),
                ..JointDesc::new(JointKind::Fixed)
            },
        },
    );
    let mut seen = None;
    for _ in 0..60 {
        tick_physics(&mut world, DT);
        let events = world.get_resource::<Events<JointBroken>>().unwrap();
        if let Some(event) = events.iter().next() {
            seen = Some(*event);
            break;
        }
    }
    let event = seen.expect("the joint should break within 60 ticks");
    assert_eq!((event.a, event.b), (a, b));
    tick_physics(&mut world, DT);
    assert!(world.get_component::<Joint>(a).is_none());
    assert!(world.get_component::<JointLink>(a).is_none());
}

#[test]
fn a_joint_to_an_entity_without_a_body_is_kept_and_retried() {
    let (mut world, a, _b) = two_bodies();
    let ghost = world.spawn();
    world.add_component(
        a,
        Joint {
            other: ghost,
            desc: JointDesc::new(JointKind::Fixed),
        },
    );
    tick_physics(&mut world, DT);
    assert!(world.get_component::<Joint>(a).is_some(), "kept");
    assert!(world.get_component::<JointLink>(a).is_none(), "not created");

    world.add_component(ghost, Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)));
    let mut ctx = world.remove_resource::<PhysicsContext>().unwrap();
    ctx.add_rigid_body(
        ghost,
        RigidBodyBuilder::dynamic().translation(Vec3::new(0.0, 2.0, 0.0)),
        vec![ColliderBuilder::ball(0.1)],
    )
    .unwrap();
    world.insert_resource(ctx);
    tick_physics(&mut world, DT);
    assert!(world.get_component::<JointLink>(a).is_some(), "retried");
}
