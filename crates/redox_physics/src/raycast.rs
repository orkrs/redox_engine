use redox_ecs::Entity;
use redox_ecs::world::World;
use redox_math::Vec3;
// Если у тебя модуль events называется по-другому, поправь импорт
use redox_ecs::event::{EventReader, Events};

use crate::context::PhysicsContext;

/// Event sent to the physics engine to request a raycast.
#[derive(Debug, Clone)]
pub struct RaycastRequest {
    /// Origin point of the ray.
    pub origin: Vec3,
    /// Direction vector of the ray (must be normalized).
    pub direction: Vec3,
    /// Maximum length the ray can travel.
    pub max_distance: f32,
    /// Custom ID to identify the request when the result comes back.
    pub request_id: u64,
    /// Optional entity to specifically notify.
    pub callback_entity: Option<Entity>,
}

/// Event produced by the physics engine containing the result of a raycast.
#[derive(Debug, Clone)]
pub struct RaycastResult {
    pub request_id: u64,
    pub hit: bool,
    pub entity: Option<Entity>,
    pub point: Vec3,
    pub normal: Vec3,
    pub distance: f32,
}

/// System that consumes `RaycastRequest` events, performs the checks,
/// and emits `RaycastResult` events.
pub fn raycast_system(context: &mut PhysicsContext, world: &mut World) {
    // 1. Читаем все входящие запросы из ресурса событий
    if let Some(events) = world.get_resource::<Events<RaycastRequest>>() {
        // Мы используем EventReader (согласно твоему коду из redox_ecs)
        let mut reader = EventReader::new(events);
        for req in reader.iter() {
            context.push_raycast_request(req.clone());
        }
    }

    // 2. Обрабатываем рейкасты через QueryPipeline физического движка
    context.process_raycast_requests(world);

    // Заметка: В `context.rs -> process_raycast_requests` тебе нужно будет
    // отправлять результаты обратно в ECS:
    // if let Some(results) = world.get_resource_mut::<Events<RaycastResult>>() {
    //     results.send(result);
    // }
}
