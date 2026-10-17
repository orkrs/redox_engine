use crate::components::{Kinematic, Velocity};
use crate::context::PhysicsContext;
use rapier3d::prelude::RigidBodyHandle;
use redox_ecs::Entity;
use redox_ecs::world::World;
use redox_math::{Quat, Transform, Vec3};
use std::cell::RefCell;

thread_local! {
    static TO_PHYSICS_SCRATCH: RefCell<Vec<(RigidBodyHandle, Option<Transform>, Option<Velocity>)>> =
        RefCell::new(Vec::with_capacity(256));
    static FROM_PHYSICS_SCRATCH: RefCell<Vec<(Entity, Vec3, Quat)>> =
        RefCell::new(Vec::with_capacity(256));
}

/// Применяет изменения из ECS в физический движок без удаления ресурса и без аллокаций в куче.
pub fn sync_to_physics(world: &mut World) {
    TO_PHYSICS_SCRATCH.with(|scratch| {
        let mut updates = scratch.borrow_mut();
        updates.clear();

        // 1. Read-only pass: query World & PhysicsContext simultaneously (both borrow &World)
        if let Some(context) = world.get_resource::<PhysicsContext>() {
            for (&entity, &body_handle) in context.active_bodies() {
                if world.get_component::<Kinematic>(entity).is_some() {
                    let transform = world.get_component::<Transform>(entity).copied();
                    updates.push((body_handle, transform, None));
                } else {
                    let velocity = world.get_component::<Velocity>(entity).copied();
                    updates.push((body_handle, None, velocity));
                }
            }
        }

        // 2. Mutable pass: apply updates to PhysicsContext
        if let Some(context) = world.get_resource_mut::<PhysicsContext>() {
            for (body_handle, transform, velocity) in updates.drain(..) {
                if let Some(t) = transform {
                    context.set_rigid_body_transform(body_handle, t.translation, t.rotation);
                } else if let Some(v) = velocity {
                    context.set_rigid_body_velocity(body_handle, v.linvel, v.angvel);
                }
            }
        }
    });
}

/// Забирает новые координаты из физического движка и обновляет ECS без удаления ресурса и без аллокаций.
pub fn sync_from_physics(world: &mut World) {
    FROM_PHYSICS_SCRATCH.with(|scratch| {
        let mut updates = scratch.borrow_mut();
        updates.clear();

        // 1. Read-only pass: read transforms from PhysicsContext for active non-kinematic bodies
        if let Some(context) = world.get_resource::<PhysicsContext>() {
            for (&entity, &body_handle) in context.active_bodies() {
                if world.get_component::<Kinematic>(entity).is_some() {
                    continue;
                }
                if let Some((pos, rot)) = context.get_rigid_body_transform(body_handle) {
                    updates.push((entity, pos, rot));
                }
            }
        }

        // 2. Mutable pass: apply updates to ECS Transform components while recording PreviousTransform
        for (entity, pos, rot) in updates.drain(..) {
            if let Some(transform) = world.get_component_mut::<Transform>(entity) {
                let current_transform = *transform;
                transform.translation = pos;
                transform.rotation = rot;

                // Update PreviousTransform if present, or add it for render interpolation
                if let Some(prev) =
                    world.get_component_mut::<crate::components::PreviousTransform>(entity)
                {
                    prev.0 = current_transform;
                }
            }
        }
    });
}

/// Interpolates between previous physics tick transform and current physics transform using alpha [0..1].
pub fn interpolate_transform(prev: &Transform, current: &Transform, alpha: f32) -> Transform {
    Transform {
        translation: prev.translation.lerp(current.translation, alpha),
        rotation: prev.rotation.slerp(current.rotation, alpha),
        scale: prev.scale.lerp(current.scale, alpha),
    }
}

/// Выполняет шаг физической симуляции (Удобная обертка)
pub fn step_physics(world: &mut World, dt: f32) {
    if let Some(context) = world.get_resource_mut::<PhysicsContext>() {
        context.step(dt);
    }
}

/// Sends contact enter/exit events into the ECS when an `Events<ContactEvent>` resource exists.
fn send_contact_events(world: &mut World, contacts: Vec<crate::contacts::ContactEvent>) {
    if contacts.is_empty() {
        return;
    }
    if let Some(events) =
        world.get_resource_mut::<redox_ecs::Events<crate::contacts::ContactEvent>>()
    {
        for contact in contacts {
            events.send(contact);
        }
    }
}

/// One physics frame: KCC, Rapier step, trigger + raycast ECS events (no resource removal).
pub fn tick_physics(world: &mut World, dt: f32) {
    crate::character::move_character_controllers(world, dt);
    step_physics(world, dt);

    let (triggers, raycasts, contacts) = {
        if let Some(context) = world.get_resource_mut::<PhysicsContext>() {
            (
                context.drain_trigger_events(),
                context.drain_raycast_results(),
                context.drain_contact_events(),
            )
        } else {
            (Vec::new(), Vec::new(), Vec::new())
        }
    };

    send_contact_events(world, contacts);

    if !triggers.is_empty() {
        if let Some(events) =
            world.get_resource_mut::<redox_ecs::Events<crate::trigger::TriggerEnter>>()
        {
            for (started, enter) in &triggers {
                if *started {
                    events.send(*enter);
                }
            }
        }
        if let Some(events) =
            world.get_resource_mut::<redox_ecs::Events<crate::trigger::TriggerExit>>()
        {
            for (started, enter) in &triggers {
                if !*started {
                    events.send(crate::trigger::TriggerExit {
                        sensor: enter.sensor,
                        other: enter.other,
                    });
                }
            }
        }
    }

    if !raycasts.is_empty()
        && let Some(events) =
            world.get_resource_mut::<redox_ecs::Events<crate::raycast::RaycastResult>>()
    {
        for res in raycasts {
            events.send(res);
        }
    }
}
