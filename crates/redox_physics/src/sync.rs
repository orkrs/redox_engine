use crate::components::{Kinematic, Velocity};
use crate::context::PhysicsContext;
use redox_ecs::world::World;

// Единый источник истины
use redox_math::Transform;

/// Применяет изменения из ECS в физический движок.
pub fn sync_to_physics(world: &mut World) {
    // Временно изымаем контекст из мира, чтобы обойти borrow checker
    let mut context = world
        .remove_resource::<PhysicsContext>()
        .expect("PhysicsContext resource is missing from the World!");

    let entities: Vec<_> = context.active_bodies().map(|(e, h)| (*e, *h)).collect();

    for (entity, body_handle) in entities {
        if world.get_component::<Kinematic>(entity).is_some() {
            if let Some(transform) = world.get_component::<Transform>(entity) {
                context.set_rigid_body_transform(
                    body_handle,
                    transform.translation,
                    transform.rotation,
                );
            }
        } else {
            if let Some(vel) = world.get_component::<Velocity>(entity) {
                context.set_rigid_body_velocity(body_handle, vel.linvel, vel.angvel);
            }
        }
    }

    // Возвращаем контекст обратно в мир
    world.insert_resource(context);
}

/// Забирает новые координаты из физического движка и обновляет ECS.
pub fn sync_from_physics(world: &mut World) {
    let context = world
        .remove_resource::<PhysicsContext>()
        .expect("PhysicsContext resource is missing from the World!");

    let mut updates = Vec::new();

    for (entity, body_handle) in context.active_bodies() {
        if world.get_component::<Kinematic>(*entity).is_some() {
            continue;
        }

        if let Some((pos, rot)) = context.get_rigid_body_transform(*body_handle) {
            updates.push((*entity, pos, rot));
        }
    }

    // Возвращаем контекст ПЕРЕД тем, как менять компоненты (чтобы мир был свободен)
    world.insert_resource(context);

    // Применяем изменения к компонентам ECS
    for (entity, pos, rot) in updates {
        if let Some(transform) = world.get_component_mut::<Transform>(entity) {
            transform.translation = pos;
            transform.rotation = rot;
        }
    }
}

/// Выполняет шаг физической симуляции (Удобная обертка)
pub fn step_physics(world: &mut World, dt: f32) {
    if let Some(context) = world.get_resource_mut::<PhysicsContext>() {
        context.step(dt);
    }
}
