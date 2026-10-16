//! Trigger overlap events from Rapier sensor colliders.

use rapier3d::prelude::CollisionEvent;
use redox_ecs::Entity;

/// A sensor overlap started between `sensor` and `other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriggerEnter {
    pub sensor: Entity,
    pub other: Entity,
}

/// A sensor overlap ended between `sensor` and `other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriggerExit {
    pub sensor: Entity,
    pub other: Entity,
}

pub(crate) fn map_collision_event(
    event: CollisionEvent,
    lookup: impl Fn(rapier3d::geometry::ColliderHandle) -> Option<Entity>,
    is_sensor: impl Fn(rapier3d::geometry::ColliderHandle) -> bool,
) -> Option<(bool, TriggerEnter)> {
    let (a, b, started) = match event {
        CollisionEvent::Started(a, b, _) => (a, b, true),
        CollisionEvent::Stopped(a, b, _) => (a, b, false),
    };
    let ea = lookup(a)?;
    let eb = lookup(b)?;
    let (sensor, other) = if is_sensor(a) {
        (ea, eb)
    } else if is_sensor(b) {
        (eb, ea)
    } else {
        return None;
    };
    Some((started, TriggerEnter { sensor, other }))
}
