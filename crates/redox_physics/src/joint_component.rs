//! ECS `Joint` components reconciled with the physics world's joints.

use crate::context::PhysicsContext;
use crate::joints::{JointBroken, JointDesc, JointId};
use redox_ecs::{Entity, Events, World};

/// Joins this entity's body with the body of `other`. Created in the physics world by [`sync_joints`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Joint {
    /// The entity whose body is joined to this entity's body.
    pub other: Entity,
    /// Kind, anchors and break limits of the joint.
    pub desc: JointDesc,
}

/// Added by [`sync_joints`] to an entity whose `Joint` exists in the physics world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JointLink(
    /// Identifier of the physics joint created for the entity's `Joint`.
    pub JointId,
);

/// Reconciles `Joint` components with the physics world; call once per physics tick before the step.
///
/// Sends a [`JointBroken`] event for each joint that broke since the previous call, creates a
/// physics joint for each `Joint` without a `JointLink` (one attempt per call; a `Joint` whose body
/// is missing is kept and retried on the next call), and removes the physics joint and the
/// components of every link whose `Joint` was removed or whose joint broke. Does nothing when the
/// world has no `PhysicsContext`.
pub fn sync_joints(world: &mut World) {
    let Some(mut physics) = world.remove_resource::<PhysicsContext>() else {
        return;
    };
    send_broken_events(world, physics.drain_joint_broken());
    create_missing_joints(world, &mut physics);
    remove_stale_joints(world, &mut physics);
    world.insert_resource(physics);
}

/// Sends the broken-joint reports into `Events<JointBroken>` when that resource exists.
fn send_broken_events(world: &mut World, broken: Vec<JointBroken>) {
    if broken.is_empty() {
        return;
    }
    if let Some(events) = world.get_resource_mut::<Events<JointBroken>>() {
        for event in broken {
            events.send(event);
        }
    }
}

/// Creates a physics joint for every `Joint` that has no `JointLink` yet.
fn create_missing_joints(world: &mut World, physics: &mut PhysicsContext) {
    let mut pending: Vec<(Entity, Joint)> = Vec::new();
    for (entity, joint) in world.query::<(Entity, &Joint)>().iter(world) {
        if world.get_component::<JointLink>(entity).is_none() {
            pending.push((entity, *joint));
        }
    }
    for (entity, joint) in pending {
        match physics.add_joint(entity, joint.other, joint.desc) {
            Ok(id) => world.add_component(entity, JointLink(id)),
            Err(err) => log::warn!(
                "Joint of {entity:?} to {:?} is not created yet: {err:?}",
                joint.other
            ),
        }
    }
}

/// Removes the physics joint and the `JointLink` of every link whose `Joint` is gone or whose joint
/// broke. A link whose joint broke also loses its `Joint`, so the entity is not joined again.
fn remove_stale_joints(world: &mut World, physics: &mut PhysicsContext) {
    let links: Vec<(Entity, JointId)> = world
        .query::<(Entity, &JointLink)>()
        .iter(world)
        .map(|(entity, link)| (entity, link.0))
        .collect();
    for (entity, id) in links {
        let has_joint = world.get_component::<Joint>(entity).is_some();
        let exists = physics.joint_exists(id);
        if has_joint && exists {
            continue;
        }
        physics.remove_joint(id);
        world.remove_component::<JointLink>(entity);
        if !exists {
            world.remove_component::<Joint>(entity);
        }
    }
}
