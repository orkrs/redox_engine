//! Virtual Geometry subsystem for RedOx Engine.
//!
//! Provides a Nanite-inspired pipeline for rendering meshes at any scale by
//! splitting geometry into small **meshlets** (clusters of ≤ 64 vertices and
//! ≤ 124 triangles) and culling / selecting LODs at the GPU level.
//!
//! ## Quick start
//!
//! ```rust,no_run
//! use redox_render::virtual_geometry::{VGSystem, VGConfig, VirtualMesh};
//! use redox_render::systems::Transform;
//!
//! // 1. Build a VG asset from an existing mesh
//! let asset_id = render_ctx.vg_system.as_mut().unwrap()
//!     .register_mesh(&my_mesh, 0, &render_ctx.queue);
//!
//! // 2. Spawn an ECS entity with a VirtualMesh component
//! let entity = world.spawn();
//! world.add_component(entity, Transform::from_translation(Vec3::ZERO));
//! world.add_component(entity, VirtualMesh::new(asset_id.unwrap()));
//!
//! // 3. In the game loop: extract, prepare, render
//! render_ctx.prepare_vg_frame(&world);
//! // render_frame() automatically draws VG objects
//! ```

pub mod asset_pipeline;
pub mod culling;
pub mod indirect;
pub mod lod;
pub mod meshlet;
pub mod runtime;

// ── Public API re-exports ─────────────────────────────────────────────────────

pub use asset_pipeline::build_vg_asset;
pub use culling::Frustum;
pub use lod::{LodChainConfig, LodLevelConfig};
pub use meshlet::{
    DrawIndexedIndirectCmd, MeshletDescriptor, VGAssetData, VGInstanceData,
    MAX_TRIANGLES_PER_MESHLET, MAX_VERTICES_PER_MESHLET,
};
pub use runtime::{VGAssetId, VGConfig, VGInstanceHandle, VGStats, VGSystem};

// ── ECS Components ────────────────────────────────────────────────────────────

/// ECS component that marks an entity as using virtual geometry.
///
/// The [`VGSystem`] reads this component to track which entities have
/// spawned VG instances and updates their transforms each frame.
#[derive(Clone, Debug)]
pub struct VirtualMesh {
    /// The VG asset to display.
    pub asset_id: VGAssetId,
    /// Handle to the spawned instance (populated by [`super::context::RenderContext::prepare_vg_frame`]).
    pub instance_handle: Option<VGInstanceHandle>,
}

impl VirtualMesh {
    /// Create a new component referencing the given asset.
    pub fn new(asset_id: VGAssetId) -> Self {
        Self {
            asset_id,
            instance_handle: None,
        }
    }
}

/// Optional per-entity VG rendering configuration.
#[derive(Clone, Debug)]
pub struct VirtualMeshConfig {
    /// LOD bias (positive = more aggressive LOD reduction).
    pub lod_bias: f32,
    /// Whether this object should cast VG shadows (future feature).
    pub cast_shadows: bool,
}

impl Default for VirtualMeshConfig {
    fn default() -> Self {
        Self {
            lod_bias: 0.0,
            cast_shadows: true,
        }
    }
}
