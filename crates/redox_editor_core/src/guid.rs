//! Lookup of entities by their stable [`EntityGuid`].
//!
//! Undo commands address entities by GUID, because an [`Entity`] handle changes when an entity
//! is deleted and created again. The resolver is a light helper that walks the world; the editor
//! scene crate keeps a faster index for its own use.

use redox_ecs::{Entity, World};
use redox_scene::components::EntityGuid;

/// Finds entities by GUID and gives GUIDs to entities that do not have one yet.
pub struct GuidResolver;

impl GuidResolver {
    /// The entity that carries `guid`, if any.
    pub fn resolve(world: &World, guid: EntityGuid) -> Option<Entity> {
        world
            .query::<(Entity, &EntityGuid)>()
            .iter(world)
            .find_map(|(entity, found)| (*found == guid).then_some(entity))
    }

    /// The GUID of `entity`, if it has one.
    pub fn guid_of(world: &World, entity: Entity) -> Option<EntityGuid> {
        world.get_component::<EntityGuid>(entity).copied()
    }

    /// The GUID of `entity`; an entity without one gets a fresh random GUID first.
    pub fn ensure(world: &mut World, entity: Entity) -> EntityGuid {
        if let Some(guid) = Self::guid_of(world, entity) {
            return guid;
        }
        let guid = EntityGuid::new_random();
        world.add_component(entity, guid);
        guid
    }
}
