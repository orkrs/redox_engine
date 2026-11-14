//! GUID-level diff between two [`SceneFile`] snapshots.

use std::collections::{BTreeMap, BTreeSet};

use redox_reflect::ReflectValue;
use redox_scene::EntityGuid;

use crate::format::{SceneEntity, SceneFile};

/// One entity-level change between an old and a new scene file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneEntityChange {
    /// Entity present in the new file but not in the old.
    Added {
        /// Stable identity of the entity.
        guid: EntityGuid,
    },
    /// Entity present in the old file but not in the new.
    Removed {
        /// Stable identity of the entity.
        guid: EntityGuid,
    },
    /// Same GUID in both files; one or more components differ.
    Changed {
        /// Stable identity of the entity.
        guid: EntityGuid,
        /// Reflect `type_name` values that were added, removed or modified.
        changed_components: Vec<String>,
    },
}

/// Compares `old` and `new` by entity GUID and reports added, removed and changed entities.
pub fn diff_scene_files(old: &SceneFile, new: &SceneFile) -> Vec<SceneEntityChange> {
    let old_map = index_by_guid(old);
    let new_map = index_by_guid(new);

    let old_keys: BTreeSet<EntityGuid> = old_map.keys().copied().collect();
    let new_keys: BTreeSet<EntityGuid> = new_map.keys().copied().collect();

    let mut out = Vec::new();

    for guid in new_keys.difference(&old_keys) {
        out.push(SceneEntityChange::Added { guid: *guid });
    }
    for guid in old_keys.difference(&new_keys) {
        out.push(SceneEntityChange::Removed { guid: *guid });
    }
    for guid in old_keys.intersection(&new_keys) {
        let changed = diff_entity_components(old_map[guid], new_map[guid]);
        if !changed.is_empty() {
            out.push(SceneEntityChange::Changed {
                guid: *guid,
                changed_components: changed,
            });
        }
    }

    out.sort_by(|a, b| change_guid(a).cmp(&change_guid(b)));
    out
}

fn change_guid(change: &SceneEntityChange) -> EntityGuid {
    match change {
        SceneEntityChange::Added { guid }
        | SceneEntityChange::Removed { guid }
        | SceneEntityChange::Changed { guid, .. } => *guid,
    }
}

fn index_by_guid(file: &SceneFile) -> BTreeMap<EntityGuid, &SceneEntity> {
    let mut map = BTreeMap::new();
    for entity in &file.entities {
        let Some(guid) = entity.guid else {
            continue;
        };
        map.insert(guid, entity);
    }
    map
}

fn diff_entity_components(old: &SceneEntity, new: &SceneEntity) -> Vec<String> {
    let old_components = component_map(old);
    let new_components = component_map(new);

    let old_names: BTreeSet<String> = old_components.keys().cloned().collect();
    let new_names: BTreeSet<String> = new_components.keys().cloned().collect();

    let mut changed = Vec::new();
    for name in new_names.difference(&old_names) {
        changed.push(name.clone());
    }
    for name in old_names.difference(&new_names) {
        changed.push(name.clone());
    }
    for name in old_names.intersection(&new_names) {
        if old_components[name.as_str()] != new_components[name.as_str()] {
            changed.push(name.clone());
        }
    }
    changed.sort();
    changed.dedup();
    changed
}

fn component_map(entity: &SceneEntity) -> BTreeMap<String, &ReflectValue> {
    let mut map = BTreeMap::new();
    for value in entity
        .components
        .iter()
        .chain(entity.unknown_components.iter())
    {
        if let ReflectValue::Struct { type_name, .. } = value {
            map.insert(type_name.clone(), value);
        }
    }
    map
}
