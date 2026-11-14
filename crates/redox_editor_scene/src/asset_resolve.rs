//! Binds runtime render handles from stored scene references after assets are imported.

use redox_asset::Handle;
use redox_ecs::World;
use redox_render::asset_types::{MaterialData, MeshData};
use redox_render::{MaterialHandle, MeshHandle};
use redox_scene::{AssetKind, AssetRefs};

/// Sets [`MeshHandle`] and [`MaterialHandle`] slots from [`AssetRefs`] GUIDs.
///
/// Unresolved GUIDs leave the default handle; the stored reference is untouched.
/// Returns how many handles were bound.
pub fn bind_render_handles(
    world: &mut World,
    mesh: impl Fn(&str) -> Option<Handle<MeshData>>,
    material: impl Fn(&str) -> Option<Handle<MaterialData>>,
) -> usize {
    let entities: Vec<_> = world.all_entities().collect();
    let mut bound = 0;
    for entity in entities {
        let pending: Vec<(AssetKind, String)> = {
            let Some(refs) = world.get_component::<AssetRefs>(entity) else {
                continue;
            };
            refs.0
                .iter()
                .filter_map(|item| {
                    let guid = item.guid.clone()?;
                    let kind = if item.kind != AssetKind::Unknown {
                        item.kind
                    } else {
                        AssetKind::from_component(&item.component)
                    };
                    Some((kind, guid))
                })
                .collect()
        };
        for (kind, guid) in pending {
            match kind {
                AssetKind::Mesh => {
                    if let Some(handle) = mesh(&guid) {
                        if let Some(existing) = world.get_component_mut::<MeshHandle>(entity) {
                            *existing = MeshHandle(handle);
                        } else {
                            world.add_component(entity, MeshHandle(handle)); // undo: n/a
                        }
                        bound += 1;
                    }
                }
                AssetKind::Material => {
                    if let Some(handle) = material(&guid) {
                        if let Some(existing) = world.get_component_mut::<MaterialHandle>(entity) {
                            *existing = MaterialHandle(handle);
                        } else {
                            world.add_component(entity, MaterialHandle(handle)); // undo: n/a
                        }
                        bound += 1;
                    }
                }
                AssetKind::Unknown => {}
            }
        }
    }
    bound
}
