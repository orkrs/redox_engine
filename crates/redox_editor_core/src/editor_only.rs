//! Marker for entities that exist only inside the editor.

use redox_ecs::{Entity, World};

/// Marks an entity that exists only inside the editor (for example a gizmo or a helper).
///
/// Such entities must never be written to a saved scene. The marker is not used for saving yet;
/// see [`editor_only_entities`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EditorOnly;

/// Returns the entities of `world` that carry [`EditorOnly`], in no particular order.
pub fn editor_only_entities(world: &World) -> Vec<Entity> {
    world
        .all_entities()
        .filter(|&entity| world.get_component::<EditorOnly>(entity).is_some())
        .collect()
}
