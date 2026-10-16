use std::collections::HashSet;

use redox_ecs::{Entity, World};
use redox_reflect::{ReflectValue, TypeRegistry, WorldSnapshot};

/// Play-In-Editor session: snapshots the world on start and restores it on stop.
///
/// Restore is in place: entities that existed before Play keep their ids (so selection,
/// undo history and the outliner stay valid), only entities spawned during Play are
/// removed. A pre-existing entity that was despawned during Play is recreated with a new id.
/// Only components registered in the [`TypeRegistry`] are snapshotted and restored.
#[derive(Clone, Debug, Default)]
pub struct PieSession {
    pub snapshot: Option<WorldSnapshot>,
    pub running: bool,
    pub paused: bool,
    existing: HashSet<Entity>,
    saved: Vec<(Entity, Vec<ReflectValue>)>,
}

impl PieSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&mut self, world: &World, registry: &TypeRegistry) {
        self.snapshot = Some(registry.snapshot_world(world));
        self.existing = world.all_entities().collect();
        self.saved = world
            .all_entities()
            .filter_map(|e| {
                let components = registry.snapshot_entity(world, e);
                (!components.is_empty()).then_some((e, components))
            })
            .collect();
        self.running = true;
        self.paused = false;
    }

    pub fn pause(&mut self) {
        self.paused = true;
    }
    pub fn resume(&mut self) {
        self.paused = false;
    }

    pub fn stop(&mut self, world: &mut World, registry: &TypeRegistry) {
        if self.snapshot.take().is_some() {
            let alive: HashSet<Entity> = world.all_entities().collect();
            for spawned in alive.difference(&self.existing) {
                world.despawn(*spawned);
            }
            for (entity, components) in std::mem::take(&mut self.saved) {
                let target = if alive.contains(&entity) {
                    entity
                } else {
                    world.spawn()
                };
                for comp in &components {
                    if let Err(e) = registry.insert_component(world, target, comp) {
                        log::warn!("PIE restore: failed to reapply component: {e:?}");
                    }
                }
            }
        }
        self.existing.clear();
        self.running = false;
        self.paused = false;
    }

    pub fn is_running(&self) -> bool {
        self.running && !self.paused
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_math::{Transform, Vec3};

    fn registry() -> TypeRegistry {
        let mut registry = TypeRegistry::new();
        registry.register::<Transform>();
        registry
    }

    #[test]
    fn stop_restores_values_and_keeps_entity_ids() {
        let registry = registry();
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)));

        let mut pie = PieSession::new();
        pie.start(&world, &registry);
        world.get_component_mut::<Transform>(e).unwrap().translation = Vec3::new(9.0, 9.0, 9.0);
        pie.stop(&mut world, &registry);

        let restored = world
            .get_component::<Transform>(e)
            .expect("same entity id survives");
        assert_eq!(restored.translation, Vec3::new(1.0, 2.0, 3.0));
        assert!(!pie.running);
    }

    #[test]
    fn stop_removes_entities_spawned_during_play() {
        let registry = registry();
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::default());

        let mut pie = PieSession::new();
        pie.start(&world, &registry);
        let spawned = world.spawn();
        world.add_component(spawned, Transform::default());
        pie.stop(&mut world, &registry);

        let remaining: Vec<_> = world.all_entities().collect();
        assert_eq!(remaining, vec![e]);
    }

    #[test]
    fn stop_recreates_entity_despawned_during_play() {
        let registry = registry();
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::from_translation(Vec3::X));

        let mut pie = PieSession::new();
        pie.start(&world, &registry);
        world.despawn(e);
        pie.stop(&mut world, &registry);

        let all: Vec<_> = world.all_entities().collect();
        assert_eq!(all.len(), 1);
        let t = world.get_component::<Transform>(all[0]).unwrap();
        assert_eq!(t.translation, Vec3::X);
    }
}
