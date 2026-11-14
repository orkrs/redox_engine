//! Runtime snapshots for shipping: editor-only entities are stripped (EDT-A-04 / TZ §23.18).

use redox_ecs::{Entity, World};
use redox_editor_core::editor_only::EditorOnly;
use redox_reflect::{EntitySnapshot, TypeRegistry, WorldSnapshot};

/// Reflected world state intended for runtime / cook (no [`EditorOnly`] entities).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct BuildSnapshot {
    /// Entities that ship to the game.
    pub world: WorldSnapshot,
}

impl BuildSnapshot {
    /// Captures every entity that is not marked [`EditorOnly`].
    pub fn from_world(registry: &TypeRegistry, world: &World) -> Self {
        let mut entities = Vec::new();
        for entity in world.all_entities() {
            if world.get_component::<EditorOnly>(entity).is_some() {
                continue;
            }
            let components = registry.snapshot_entity(world, entity);
            if components.is_empty() {
                continue;
            }
            entities.push(EntitySnapshot { components });
        }
        Self {
            world: WorldSnapshot { entities },
        }
    }

    /// Entity count in the snapshot (equals runtime entities).
    pub fn entity_count(&self) -> usize {
        self.world.entities.len()
    }

    /// Whether `entity` would be omitted from a build snapshot.
    pub fn is_stripped(world: &World, entity: Entity) -> bool {
        world.get_component::<EditorOnly>(entity).is_some()
    }
}
