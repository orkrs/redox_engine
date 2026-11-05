//! Two-way index between scene entities and their [`EntityGuid`].
//!
//! The editor keeps the index up to date for the operations it performs (`insert`,
//! `remove_entity`); `rebuild` rescans the world and is the reference state.

use std::collections::HashMap;

use redox_ecs::{Entity, World};
use redox_scene::EntityGuid;

/// Maps each entity to its guid and back.
#[derive(Debug, Default, Clone)]
pub struct GuidIndex {
    by_guid: HashMap<EntityGuid, Entity>,
    by_entity: HashMap<Entity, EntityGuid>,
}

impl GuidIndex {
    /// Creates an empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the index with the guids of every entity in `world` that has one.
    pub fn rebuild(&mut self, world: &World) {
        self.by_guid.clear();
        self.by_entity.clear();
        for entity in world.all_entities() {
            if let Some(guid) = world.get_component::<EntityGuid>(entity).copied() {
                self.insert(guid, entity);
            }
        }
    }

    /// Records that `entity` has `guid`; an older mapping of either side is dropped.
    pub fn insert(&mut self, guid: EntityGuid, entity: Entity) {
        self.remove_entity(entity);
        if let Some(previous) = self.by_guid.insert(guid, entity) {
            self.by_entity.remove(&previous);
        }
        self.by_entity.insert(entity, guid);
    }

    /// Forgets `entity` (call it when the entity is despawned).
    pub fn remove_entity(&mut self, entity: Entity) {
        if let Some(guid) = self.by_entity.remove(&entity) {
            self.by_guid.remove(&guid);
        }
    }

    /// The entity that has `guid`, if it is indexed.
    pub fn get(&self, guid: EntityGuid) -> Option<Entity> {
        self.by_guid.get(&guid).copied()
    }

    /// The guid of `entity`, if it is indexed.
    pub fn guid_of(&self, entity: Entity) -> Option<EntityGuid> {
        self.by_entity.get(&entity).copied()
    }

    /// Number of indexed entities.
    pub fn len(&self) -> usize {
        self.by_guid.len()
    }

    /// `true` when no entity is indexed.
    pub fn is_empty(&self) -> bool {
        self.by_guid.is_empty()
    }
}

/// Gives a fresh [`EntityGuid`] to every entity of `world` that has none.
///
/// Returns how many entities received one. Saving calls this, so that a scene is always saved
/// with a complete set of ids.
pub fn assign_missing_guids(world: &mut World) -> usize {
    let missing: Vec<Entity> = world
        .all_entities()
        .filter(|entity| world.get_component::<EntityGuid>(*entity).is_none())
        .collect();
    for entity in &missing {
        world.add_component(*entity, EntityGuid::new_random()); // undo: init
    }
    missing.len()
}
