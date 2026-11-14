//! Runtime entity search by name or GUID during PIE.

use redox_ecs::{Entity, World};
use redox_scene::{Name, components::EntityGuid};

/// Match criteria for the runtime inspector search box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuntimeSearchQuery {
    /// Lowercase needle matched against entity names and GUID strings.
    pub text: String,
}

/// One hit in the runtime search list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeSearchHit {
    /// ECS entity.
    pub entity: Entity,
    /// Display name, if any.
    pub name: Option<String>,
    /// Stable GUID string, if any.
    pub guid: Option<String>,
}

/// Searches `world` for entities matching `query`.
pub fn search_runtime_entities(world: &World, query: &RuntimeSearchQuery) -> Vec<RuntimeSearchHit> {
    let needle = query.text.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for entity in world.all_entities() {
        let name = world
            .get_component::<Name>(entity)
            .map(|n| n.0.to_ascii_lowercase());
        let guid = world
            .get_component::<EntityGuid>(entity)
            .map(|g| format!("{:032x}", g.0));
        let name_hit = name.as_ref().is_some_and(|n| n.contains(&needle));
        let guid_hit = guid.as_ref().is_some_and(|g| g.contains(&needle));
        if name_hit || guid_hit {
            hits.push(RuntimeSearchHit {
                entity,
                name: world.get_component::<Name>(entity).map(|n| n.0.clone()),
                guid,
            });
        }
    }
    hits
}

/// Viewport overlay flag: highlight this entity during PIE (consumed by B3 integrator).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RuntimeInspectorHighlight {
    /// Entity to outline in the viewport; `None` clears the highlight.
    pub entity: Option<Entity>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_scene::components::EntityGuid;

    #[test]
    fn search_finds_name_and_guid_without_ui() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Name("Hero".into()));
        world.add_component(e, EntityGuid(0xabc));
        let hits = search_runtime_entities(
            &world,
            &RuntimeSearchQuery {
                text: "hero".into(),
            },
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entity, e);

        let guid_hits = search_runtime_entities(
            &world,
            &RuntimeSearchQuery {
                text: "00000000000000000000000000000abc".into(),
            },
        );
        assert_eq!(guid_hits.len(), 1);
    }
}