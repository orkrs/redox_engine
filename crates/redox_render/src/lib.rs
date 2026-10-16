//! Rendering subsystem for the RedOx Engine.
//!
//! This crate provides a `wgpu`-based renderer with forward shading,
//! procedural mesh generation, texture loading, and ECS integration
//! through the `redox_ecs` crate.

pub mod asset_types;
pub mod camera;
pub mod clustering;
pub mod cluster_manager;
pub mod cluster_gpu_cull;
pub mod context;
pub mod frame;
pub mod light;
pub mod registry;
pub mod renderer;
pub mod material;
pub mod mesh;
pub mod pass;
pub mod post;
pub mod gi;
pub mod resource;
pub mod sdf;
pub mod shader;
pub mod profiler;
pub mod shadow;
pub mod skinning;
pub mod systems;
pub mod virtual_geometry;

pub use camera::{ActiveCamera, Camera, CameraUniform, PreviousViewProj};
pub use context::RenderContext;
pub use frame::FrameView;
pub use renderer::{Renderer, pick_hdr_msaa_sample_count};
pub use light::{DirectionalLight, LightScene, LightUniform, LightingMode, PointLight, PointLightGpu, SpotLight};
pub use material::Material;
pub use asset_types::{BlendMode, MaterialData, MaterialLayer, MeshData, TextureData};
pub use mesh::{Mesh, Vertex};
pub use profiler::{GpuProfiler, GpuTiming, PROFILER_FEATURES};
pub use skinning::SkinnedMeshBinding;
pub use systems::{FrameRenderObjects, MaterialHandle, MeshHandle, PreviousTransform, RenderObject, Transform};
pub use clustering::ClusterInfo;
pub use cluster_manager::{ClusterManager, ClusterAssignmentStats};
// TAA public surface
pub use pass::taa::{halton_jitter_ndc, TaaUniform};
// Skybox and transparency passes
pub use pass::skybox::SkyboxPass;
pub use pass::shared_layouts::{
    ForwardPass, ModelUniform, PbrBindGroupPlan, PbrFrameLayouts, PbrSceneLayouts, SharedLayouts,
};
pub use pass::particles::ParticleGpuSystem;
pub use pass::transparent::TransparentPass;
// Virtual Geometry visibility-buffer shading (Phase 3)
pub use pass::vg_resolve::{
    bind_vis_draw_state, build_material_storage, build_meshlet_storage,
    VgResolvePass, VgVisPass, VIS_ID_FORMAT, VIS_ID_SENTINEL,
    VIS_NORMAL_DEPTH_FORMAT, VIS_UV_FORMAT,
};
// Tone mapping (ACES filmic + exposure)
pub use post::tone_mapping::{
    TonemapUniform, create_tonemap_bind_group, create_tone_mapping_pipeline,
    create_rcas_present_pipeline,
};
// Sharp AA passes (MSAA smart resolve + SMAA 1×)
pub use pass::msaa_resolve::MsaaResolvePass;
pub use pass::smaa::SmaaPass;
// Post-processing scaffolds (Phase 5)
pub use post::bloom::BloomPass;
// Global illumination (Phase 5)
pub use gi::{DdgiProbe, DdgiProbeGrid};
// ScatterLight stochastic lighting (Phase 4)
pub use pass::scatter_light::{
    ScatterLightPass, ScatterLightUniform, SCATTER_LIGHT_SAMPLE_COUNT,
};
// Virtual Geometry
pub use virtual_geometry::{
    VGAssetId, VGConfig, VGInstanceHandle, VGStats, VGSystem, VGAssetData,
    VirtualMesh, VirtualMeshConfig,
};
// Shadow / VSM quality presets
pub use shadow::{SmrtConfig, SmrtQuality};
// SDF foundation (distance fields, brick registry, cone-trace reference)
pub use sdf::{DistanceField64, SdfBrickAtlas, SdfShadowScene, bake_mesh};
