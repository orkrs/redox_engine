use std::collections::HashSet;

use redox_ecs::{Entity, World};
use redox_reflect::{ReflectValue, TypeRegistry, WorldSnapshot};

/// Whether PIE runs as full Play or Simulate (no gameplay input focus).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PieRunMode {
    /// Gameplay systems receive input focus while running.
    #[default]
    Play,
    /// One-frame / editor-driven simulation without gameplay input focus.
    Simulate,
}

/// Play-In-Editor session: snapshots the world on start and restores it on stop.
///
/// Restore is in place: entities that existed before Play keep their ids (so selection,
/// undo history and the outliner stay valid), only entities spawned during Play are
/// removed. A pre-existing entity that was despawned during Play is recreated with a new id.
/// Only components registered in the [`TypeRegistry`] are snapshotted and restored.
#[derive(Clone, Debug, Default)]
pub struct PieSession {
    /// World snapshot taken when play started, restored on stop.
    pub snapshot: Option<WorldSnapshot>,
    /// Whether the game is running.
    pub running: bool,
    /// Whether a running game is paused.
    pub paused: bool,
    /// Play vs Simulate (input focus).
    pub run_mode: PieRunMode,
    /// Number of logical frames advanced while running (step + live ticks).
    pub frame_count: u64,
    existing: HashSet<Entity>,
    saved: Vec<(Entity, Vec<ReflectValue>)>,
}

impl PieSession {
    /// Creates a stopped session.
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts playing: snapshots `world` so it can be restored later.
    pub fn start(&mut self, world: &World, registry: &TypeRegistry) {
        self.start_with_mode(world, registry, PieRunMode::Play);
    }

    /// Starts PIE in the given [`PieRunMode`].
    pub fn start_with_mode(&mut self, world: &World, registry: &TypeRegistry, mode: PieRunMode) {
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
        self.run_mode = mode;
        self.frame_count = 0;
    }

    /// Pauses the running game.
    pub fn pause(&mut self) {
        self.paused = true;
    }
    /// Resumes a paused game.
    pub fn resume(&mut self) {
        self.paused = false;
    }

    /// Advances exactly one logical frame while [`Self::running`] is true.
    ///
    /// `tick` is the host hook that runs one dispatcher frame (or a test stub).
    /// The frame counter increases by one on every successful step.
    pub fn step(
        &mut self,
        world: &mut World,
        registry: &TypeRegistry,
        tick: &mut impl FnMut(&mut World, &TypeRegistry),
    ) {
        if !self.running {
            return;
        }
        tick(world, registry);
        self.frame_count = self.frame_count.saturating_add(1);
    }

    /// Stops playing and restores the snapshot taken at start.
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
        self.frame_count = 0;
    }

    /// Whether live simulation should advance (not paused).
    pub fn is_running(&self) -> bool {
        self.running && !self.paused
    }

    /// Whether gameplay input should be routed to PIE (Play mode and live).
    pub fn gameplay_input_enabled(&self) -> bool {
        self.run_mode == PieRunMode::Play && self.is_running()
    }

    /// Persistent scene/world files must not be written while PIE holds a snapshot.
    pub fn blocks_persistent_save(&self) -> bool {
        self.running
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

    #[test]
    fn n_steps_increase_frame_count_by_n() {
        let registry = registry();
        let mut world = World::new();
        let mut pie = PieSession::new();
        pie.start(&world, &registry);
        let steps = 5usize;
        let mut calls = 0u32;
        for _ in 0..steps {
            pie.step(&mut world, &registry, &mut |_, _| calls += 1);
        }
        assert_eq!(pie.frame_count, steps as u64);
        assert_eq!(calls, steps as u32);
    }

    #[test]
    fn stop_after_steps_restores_hierarchy_and_values() {
        let registry = registry();
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::from_translation(Vec3::Y));

        let mut pie = PieSession::new();
        pie.start(&world, &registry);
        for _ in 0..3 {
            pie.step(&mut world, &registry, &mut |w, _| {
                if let Some(t) = w.get_component_mut::<Transform>(e) {
                    t.translation.x += 1.0;
                }
            });
        }
        assert_eq!(pie.frame_count, 3);
        pie.stop(&mut world, &registry);
        let t = world.get_component::<Transform>(e).unwrap();
        assert_eq!(t.translation, Vec3::Y);
    }

    #[test]
    fn simulate_mode_disables_gameplay_input_while_play_does_not() {
        let registry = registry();
        let world = World::new();
        let mut pie = PieSession::new();
        pie.start_with_mode(&world, &registry, PieRunMode::Simulate);
        assert!(!pie.gameplay_input_enabled());
        pie.stop(&mut World::new(), &registry);

        pie.start_with_mode(&world, &registry, PieRunMode::Play);
        assert!(pie.gameplay_input_enabled());
        pie.pause();
        assert!(!pie.gameplay_input_enabled());
    }
}
