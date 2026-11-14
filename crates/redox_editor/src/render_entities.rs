//! World → [`ViewportDrawItem`] for the engine render host.

use redox_ecs::World;
use redox_editor_ui::EditorMeshProxy;
use redox_editor_viewport::{ViewportDrawItem, ViewportSceneAssetRef};
use redox_math::Transform;
use redox_scene::AssetRefs;

/// Collects every visible entity with a transform for viewport rendering.
pub fn collect_viewport_draws(world: &World) -> Vec<ViewportDrawItem> {
    let mut out = Vec::new();
    for entity in world.all_entities() {
        let Some(transform) = world.get_component::<Transform>(entity).copied() else {
            continue;
        };
        if transform.translation.y < -0.05 {
            continue;
        }
        let mesh_path = world
            .get_component::<EditorMeshProxy>(entity)
            .map(|p| p.path.clone());
        let mesh_ref = world
            .get_component::<AssetRefs>(entity)
            .and_then(|refs| refs.get("MeshHandle"))
            .and_then(ViewportSceneAssetRef::from_scene_ref);
        out.push(ViewportDrawItem {
            entity_id: entity.id(),
            transform,
            mesh_path,
            mesh_ref,
        });
    }
    out
}

/// Merges scene [`AssetRefs`] when spawning from the asset browser.
pub fn stamp_mesh_ref(world: &mut World, entity: redox_ecs::Entity, path: &std::path::Path) {
    if world.get_component::<AssetRefs>(entity).is_none() {
        world.add_component(entity, AssetRefs::default()); // undo: tx
    }
    let rel = path
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_string);
    if let Some(refs) = world.get_component_mut::<AssetRefs>(entity) {
        refs.set("MeshHandle", None, rel);
    }
}
