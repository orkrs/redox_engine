//! Rendering subsystem for the RedOx Engine.
//!
//! This crate provides a `wgpu`-based renderer with forward shading,
//! procedural mesh generation, texture loading, and ECS integration
//! through the `redox_ecs` crate.

#![warn(missing_docs)]

pub mod asset_types;
pub mod camera;
pub mod cluster_gpu_cull;
pub mod cluster_manager;
pub mod clustering;
pub mod context;
pub mod frame;
pub mod frame_graph;
pub mod gi;
pub mod light;
pub mod material;
/// GPU memory bookkeeping by category (pure accounting, no GPU access).
pub mod memory_budget;
pub mod mesh;
pub mod pass;
pub mod pipeline_cache;
/// Post-processing passes.
pub mod post;
pub mod profiler;
pub mod registry;
pub mod render_graph;
pub mod renderer;
pub mod resource;
pub mod sdf;
pub mod shader;
pub mod shadow;
/// Substrate-lite material model: slabs, clearcoat, energy checks (MAT-04).
pub mod substrate;
pub mod skinning;
mod skinning_gpu;
pub mod surface;
pub mod systems;
pub mod virtual_geometry;

pub use asset_types::{BlendMode, MaterialData, MaterialLayer, MeshData, TextureData};
pub use camera::{ActiveCamera, Camera, CameraUniform, PreviousViewProj};
pub use cluster_manager::{ClusterAssignmentStats, ClusterManager};
pub use clustering::ClusterInfo;
pub use context::RenderContext;
pub use frame::FrameView;
pub use light::{
    DirectionalLight, LightScene, LightUniform, LightingMode, PointLight, PointLightGpu, SpotLight,
};
pub use material::Material;
pub use memory_budget::{
    CategoryUsage, MemoryBudget, MemoryCategory, texture_bytes_blocks, texture_size_bytes,
};
pub use mesh::{Mesh, Vertex};
pub use profiler::{GpuProfiler, GpuTiming, PROFILER_FEATURES};
pub use renderer::{Renderer, RendererError, pick_hdr_msaa_sample_count};
pub use skinning::SkinnedMeshBinding;
pub use surface::{SurfaceError, acquire_surface_texture};
pub use systems::{
    FrameRenderObjects, MaterialHandle, MeshHandle, PreviousTransform, RenderObject, Transform,
};
// TAA public surface
pub use pass::taa::{TaaUniform, halton_jitter_ndc};
// Skybox and transparency passes
pub use pass::particles::ParticleGpuSystem;
pub use pass::shared_layouts::{
    ForwardPass, ModelUniform, PbrBindGroupPlan, PbrFrameLayouts, PbrSceneLayouts, SharedLayouts,
};
pub use pass::skybox::SkyboxPass;
pub use pass::transparent::TransparentPass;
// Virtual Geometry visibility-buffer shading (Phase 3)
pub use pass::vg_resolve::{
    VIS_ID_FORMAT, VIS_ID_SENTINEL, VIS_NORMAL_DEPTH_FORMAT, VIS_UV_FORMAT, VgResolvePass,
    VgVisPass, bind_vis_draw_state, build_material_storage, build_meshlet_storage,
};
// Tone mapping (ACES filmic + exposure)
pub use post::tone_mapping::{
    TonemapUniform, create_rcas_present_pipeline, create_tone_mapping_pipeline,
    create_tonemap_bind_group,
};
// Sharp AA passes (MSAA smart resolve + SMAA 1×)
pub use pass::msaa_resolve::MsaaResolvePass;
pub use pass::smaa::SmaaPass;
// Post-processing scaffolds (Phase 5)
pub use post::bloom::BloomPass;
// Global illumination (Phase 5)
pub use gi::{DdgiProbe, DdgiProbeGrid};
// ScatterLight stochastic lighting (Phase 4)
pub use pass::scatter_light::{SCATTER_LIGHT_SAMPLE_COUNT, ScatterLightPass, ScatterLightUniform};
// Virtual Geometry
pub use virtual_geometry::{
    VGAssetData, VGAssetId, VGConfig, VGInstanceHandle, VGStats, VGSystem, VirtualMesh,
    VirtualMeshConfig,
};
// Shadow / VSM quality presets
pub use shadow::{SmrtConfig, SmrtQuality};
// SDF foundation (distance fields, brick registry, cone-trace reference)
pub use sdf::{DistanceField64, SdfBrickAtlas, SdfShadowScene, bake_mesh};
