//! ECS components and systems for render integration.
//!
//! Uses [`redox_asset::Handle`] for [`MeshHandle`] and [`MaterialHandle`];
//! sync assets to the render context (e.g. [`sync_assets_to_render`]) before
//! calling [`extract_render_objects`].

use redox_asset::Handle;
use redox_ecs::World;
use redox_math::Mat4;

use crate::asset_types::{MaterialData, MeshData};
use crate::context::RenderContext;

pub use redox_math::Transform;

// ---------------------------------------------------------------------------
// Components (handle-based)
// ---------------------------------------------------------------------------

/// Handle to a mesh asset. Resolved to a GPU mesh index by [`RenderContext`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshHandle(pub Handle<MeshData>);

/// Stores the **previous frame's model matrix** for TAA velocity generation.
///
/// Add this component alongside [`Transform`] on any entity that should
/// contribute accurate per-object motion vectors.  Update it once per frame
/// (after rendering, before moving the object):
/// ```rust,ignore
/// // at the end of the Update stage
/// if let Some(prev) = world.get_component_mut::<PreviousTransform>(entity) {
///     prev.matrix = current_transform.matrix();
/// }
/// ```
/// Without this component the velocity pass assumes the object was stationary,
/// which is correct for static geometry but may produce slight ghosting on fast
/// moving objects.
#[derive(Clone, Copy, Debug)]
pub struct PreviousTransform {
    /// Model matrix from the previous frame.
    pub matrix: Mat4,
}

impl PreviousTransform {
    pub fn new(matrix: Mat4) -> Self {
        Self { matrix }
    }
}

/// Handle to a material asset. Resolved to a GPU material index by [`RenderContext`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaterialHandle(pub Handle<MaterialData>);

// ---------------------------------------------------------------------------
// Per-frame render data
// ---------------------------------------------------------------------------

/// Temporary structure built each frame during the `RenderPrep` stage.
#[derive(Clone, Debug)]
pub struct RenderObject {
    pub model_matrix: Mat4,
    pub color: [f32; 4],
    pub mesh_index: usize,
    pub material_index: usize,
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Extracts render data from the ECS world. Entities with [`MeshHandle`] and
/// [`MaterialHandle`] are included only if the render context has already
/// uploaded those assets (e.g. after [`sync_assets_to_render`]).
pub fn extract_render_objects(
    world: &World,
    render_context: &RenderContext,
) -> Vec<RenderObject> {
    let mut render_objects = Vec::new();

    for e in world.all_entities() {
        let (Some(transform), Some(mesh), Some(material)) = (
            world.get_component::<Transform>(e),
            world.get_component::<MeshHandle>(e),
            world.get_component::<MaterialHandle>(e),
        ) else {
            continue;
        };

        let mesh_index = match render_context.get_mesh_index(mesh.0) {
            Some(i) => i,
            None => continue,
        };
        let material_index = match render_context.get_material_index(material.0) {
            Some(i) => i,
            None => continue,
        };

        let mut obj_color = [1.0, 1.0, 1.0, 1.0];
        if let Some(mat) = render_context.materials.get(material_index) {
            obj_color = [mat.base_color.x, mat.base_color.y, mat.base_color.z, 1.0];
        }

        render_objects.push(RenderObject {
            model_matrix: transform.matrix(),
            mesh_index,
            material_index,
            color: obj_color,
        });
    }

    render_objects
}

/// Syncs loaded assets from the asset manager into the render context.
///
/// Call each frame after `asset_manager.update(world)`. For each handle in the
/// given slices, if the asset is ready and not yet in the render context, uploads
/// it (mesh, texture, or material). Texture handles should be synced before
/// material handles that reference them.
pub fn sync_assets_to_render(
    render_ctx: &mut RenderContext,
    asset_manager: &redox_asset::AssetManager,
    mesh_handles: &[Handle<MeshData>],
    texture_handles: &[Handle<crate::asset_types::TextureData>],
    material_handles: &[Handle<MaterialData>],
) {
    for &handle in mesh_handles {
        if render_ctx.get_mesh_index(handle).is_some() {
            continue;
        }
        if let Some(mesh) = asset_manager.get(handle) {
            render_ctx.add_mesh_from_asset(handle, mesh);
        }
    }

    for &handle in texture_handles {
        if render_ctx.get_texture_index(handle).is_some() {
            continue;
        }
        if let Some(img) = asset_manager.get(handle) {
            let _ = render_ctx.add_texture_from_asset(
                handle,
                img,
                &format!("tex_{:?}", handle.id()),
            );
        }
    }

    for &handle in material_handles {
        if render_ctx.get_material_index(handle).is_some() {
            continue;
        }
        if let Some(data) = asset_manager.get(handle) {
            render_ctx.add_material_from_asset(handle, data);
        }
    }
}
