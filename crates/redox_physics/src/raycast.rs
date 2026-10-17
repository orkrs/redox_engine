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
    /// Id of the request this result answers.
    pub request_id: u64,
    /// Whether the ray hit something within range.
    pub hit: bool,
    /// Entity that was hit, if it is mapped to one.
    pub entity: Option<Entity>,
    /// World-space hit point (the ray origin when nothing was hit).
    pub point: Vec3,
    /// Surface normal facing the ray (zero when nothing was hit).
    pub normal: Vec3,
    /// Distance travelled along the ray (the maximum distance when nothing was hit).
    pub distance: f32,
}

/// System that consumes `RaycastRequest` events, performs the checks,
/// and emits `RaycastResult` events.
pub fn raycast_system(
    reader: &mut EventReader<RaycastRequest>,
    context: &mut PhysicsContext,
    world: &mut World,
) {
    // 1. Читаем все входящие запросы из ресурса событий
    if let Some(events) = world.get_resource::<Events<RaycastRequest>>() {
        for req in reader.iter(events) {
            context.push_raycast_request(req.clone());
        }
    }

    // 2. Обрабатываем рейкасты через QueryPipeline физического движка
    context.process_raycast_requests(world);
}
