//! Core renderer: owns wgpu device/queue/surface, passes, and GPU registry.

mod aa;
mod accessors;
mod assets;
mod attachments;
mod contexts;
mod debug_draw;
mod draw;
mod frame;
mod helpers;
mod init;
pub mod motion;
mod passes;
mod post;
mod reload;
pub use reload::{ReloadablePass, ShaderHotReload};
mod resize;
mod scene_sync;
mod settings;

pub use helpers::pick_hdr_msaa_sample_count;
use helpers::*;

use bytemuck;
use std::sync::Arc;
use winit::window::Window;

use rayon::join;
use redox_asset::Handle;
use redox_vfx::{ParticleSimUniforms, ParticleSystem};

use contexts::{
    AtlasCasterJob, AtlasContext, PrepassContext, VsmContext, compute_main_visibility,
    record_normal_prepass, record_point_atlas_shadows, record_vsm_pages,
};

use crate::asset_types::{BlendMode, MaterialData, MeshData, TextureData};
use crate::camera::CameraUniform;
use crate::cluster_manager::ClusterManager;
use crate::light::{LightUniform, LightingMode, ShaderDebugUniform};

/// Debug visualization mode for the PBR shader (binding 21).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum ShaderDebugVizMode {
    /// Debug visualization off.
    #[default]
    None = 0,
    /// Shows the light cluster index.
    ClusterIndex = 1,
    /// Shows the shadow value.
    ShadowValue = 2,
    /// Magenta=SSAO · Lime=VSM · Yellow=SDF (darkening amounts).
    ShadowSources = 3,
}
use crate::camera::Camera;
use crate::frame::FrameView;
use crate::material::Material;
use crate::mesh::{Mesh, bounds_vectors, compress_vertices};
use crate::pass::debug_lines::DebugLinesPass;
use crate::pass::msaa_resolve::MsaaResolvePass;
use crate::pass::normal::NormalPass;
use crate::pass::particles::ParticleGpuSystem;
use crate::pass::pbr::PbrPass;
use crate::pass::scatter_light::{ScatterLightPass, ScatterLightUniform};
use crate::pass::shadow::{SHADOW_FORMAT, SHADOW_SIZE, ShadowPass};
use crate::pass::shared_layouts::{
    ModelUniform, PbrFrameLayouts, SharedLayouts, create_depth_texture,
};
use crate::pass::skybox::SkyboxPass;
use crate::pass::smaa::SmaaPass;
use crate::pass::taa::{
    TaaPass, TaaUniform, apply_jitter_to_projection, create_history_texture,
    create_velocity_texture, halton_jitter_ndc,
};
use crate::pass::transparent::TransparentPass;
use crate::pass::velocity::{ObjectVelocityPass, VelocityPass, VelocityUniform};
use crate::pass::vg_resolve::{
    VIS_ID_SENTINEL, VgResolvePass, VgShadowPass, VgVisPass, VgVisTargets, build_material_storage,
    create_vg_vis_targets,
};
use crate::post::bloom::BloomPass;
use crate::post::ssao::{SSAOPass, VsmPassUniform, VsmShadowPass, VsmTemporalUniform};
use crate::post::tone_mapping::{
    TonemapUniform, create_rcas_present_pipeline, create_tone_mapping_pipeline,
    create_tonemap_bind_group,
};
use crate::registry::{GpuMesh, GpuRegistry};
use crate::resource::buffer;
use crate::resource::ibl::{IBLProcessor, IBLResource};
use crate::resource::texture::Texture;
use crate::systems::RenderObject;
use crate::systems::Transform;
use crate::virtual_geometry::hiz::HiZPyramid;
use crate::virtual_geometry::runtime::{VGConfig, VGSystem};
use crate::virtual_geometry::{VGAssetId, VirtualMesh};
use redox_math::{Aabb, Mat3, Mat4, Vec3};

/// HDR geometry pass that owns the MSAA resolve when multisampling is active.
#[derive(Copy, Clone, Eq, PartialEq)]
enum HdrGeometryResolvePass {
    Forward,
    VgResolve,
    VgDebug,
    Transparent,
    /// Sharp AA: no hardware resolve — all passes Store, MsaaResolvePass runs afterwards.
    SmartResolve,
}

/// Central GPU renderer — owns device, passes, frame state, and [`GpuRegistry`].
pub struct Renderer {
    // --- wgpu core ---
    /// The wgpu device.
    pub device: wgpu::Device,
    /// The wgpu queue.
    pub queue: wgpu::Queue,
    /// Window swapchain. `None` for offscreen editor hosts that only call
    /// [`Self::render_frame_into`] (egui SharedGpu owns the real surface).
    pub surface: Option<wgpu::Surface<'static>>,
    /// Current surface configuration.
    pub config: wgpu::SurfaceConfiguration,
    /// Internal render scale for TAAU (render at lower res, upscale temporally).
    pub render_scale: f32,
    /// Internal render width in pixels.
    pub internal_width: u32,
    /// Internal render height in pixels.
    pub internal_height: u32,

    // --- Render pass ---
    /// Bind group layouts shared by several passes.
    pub shared_layouts: SharedLayouts,
    /// Planned decoupled PBR frame bind groups (Phase 2.2 factories).
    pub pbr_frame_layouts: PbrFrameLayouts,
    /// Main PBR forward pass.
    pub pbr_pass: PbrPass,
    /// Shadow depth pass.
    pub shadow_pass: ShadowPass,
    /// Pass that renders normals and depth.
    pub normal_pass: NormalPass,
    /// Ambient occlusion passes.
    pub ssao_pass: SSAOPass,

    // --- Global Uniforms ---
    /// GPU buffer with the camera uniform.
    pub camera_buffer: wgpu::Buffer,
    /// CPU copy of the camera uniform.
    pub camera_uniform: CameraUniform,
    /// Near/far of the last uploaded camera — must match `camera_uniform.proj`
    /// for the VSM clipmap and cluster setup.
    pub camera_near: f32,
    /// Far plane distance of the main camera.
    pub camera_far: f32,

    /// CPU copy of the light uniform.
    pub light_uniform: LightUniform,
    /// GPU buffer with the light uniform.
    pub light_buffer: wgpu::Buffer,
    /// GPU buffer with the shader debug uniform.
    pub shader_debug_buffer: wgpu::Buffer,

    /// Bind group with per-frame data (camera).
    pub frame_bind_group: wgpu::BindGroup,
    /// Bind group with scene data (lights, shadows, environment).
    pub scene_bind_group: wgpu::BindGroup,

    // --- HDR & Post-processing ---
    /// Multisample count for HDR geometry (2× when TAA is enabled).
    pub msaa_sample_count: u32,
    /// Preferred HDR MSAA count when TAA is on (queried from adapter at init).
    hdr_msaa_target_count: u32,
    /// HDR color target the scene is shaded into.
    pub hdr_texture: wgpu::Texture,
    /// View of the HDR target.
    pub hdr_view: wgpu::TextureView,
    /// 2× multisampled HDR colour target for geometry passes.
    pub hdr_msaa_texture: wgpu::Texture,
    /// Multisampled HDR view used when MSAA is on.
    pub hdr_msaa_view: wgpu::TextureView,
    /// Pipeline that tone-maps the HDR image to the surface.
    pub tone_mapping_pipeline: wgpu::RenderPipeline,
    /// Bind group of the tone-mapping pass.
    pub tone_mapping_bind_group: wgpu::BindGroup,
    /// Uniform for the tone-mapping pass (`TonemapUniform`: exposure /
    /// sharpen toggle / output size). Written each frame before submission.
    pub tonemap_uniform_buffer: wgpu::Buffer,
    /// Linear exposure multiplier fed to the ACES tone mapper (default 1.0).
    pub exposure: f32,
    /// RCAS sharpness in stops (lower = sharper post-TAA recovery).
    pub rcas_sharpness: f32,
    /// Sampler for reading the HDR image.
    pub hdr_sampler: wgpu::Sampler,

    // --- Normal pass + SSAO ---
    /// View-space normal target.
    pub normal_texture: wgpu::Texture,
    /// View of the normal target.
    pub normal_view: wgpu::TextureView,
    /// Bind group for reading the normal target.
    pub normal_bind_group: wgpu::BindGroup,
    /// Raw ambient occlusion (half resolution).
    pub ssao_raw_texture: wgpu::Texture,
    /// View of the raw occlusion texture.
    pub ssao_raw_view: wgpu::TextureView,
    /// Half-res bilateral blur output (fed into the upsample pass).
    pub ssao_half_blurred_texture: wgpu::Texture,
    /// View of the half-resolution blurred occlusion.
    pub ssao_half_blurred_view: wgpu::TextureView,
    /// Full-resolution blurred occlusion texture.
    pub ssao_blurred_texture: wgpu::Texture,
    /// View of the full-resolution occlusion.
    pub ssao_blurred_view: wgpu::TextureView,
    /// Bind group of the occlusion trace.
    pub ssao_bind_group: wgpu::BindGroup,
    /// Bind group of the occlusion blur.
    pub ssao_blur_bind_group: wgpu::BindGroup,
    /// Bind group of the occlusion upsample.
    pub ssao_upsample_bind_group: wgpu::BindGroup,
    /// Sampler for sampling SSAO texture in PBR (Nearest — matches filterable: false / R16Float).
    pub ssao_pbr_sampler: wgpu::Sampler,
    /// When false, GTAO passes are skipped and the AO texture stays white (1.0).
    pub ssao_enabled: bool,

    // --- VSM screen-space shadow (1-ray + temporal) ---
    vsm_shadow_pass: VsmShadowPass,
    vsm_shadow_raw_texture: wgpu::Texture,
    vsm_shadow_raw_view: wgpu::TextureView,
    vsm_shadow_filtered_texture: wgpu::Texture,
    vsm_shadow_filtered_view: wgpu::TextureView,
    vsm_shadow_history_texture: wgpu::Texture,
    vsm_shadow_history_view: wgpu::TextureView,
    vsm_shadow_trace_bind_group: wgpu::BindGroup,
    vsm_shadow_temporal_bind_group: wgpu::BindGroup,
    vsm_shadow_pbr_sampler: wgpu::Sampler,

    // --- Depth ---
    /// Scene depth texture.
    #[allow(dead_code)]
    pub depth_texture: wgpu::Texture,
    /// View of the scene depth.
    pub depth_view: wgpu::TextureView,
    /// Multisampled depth for HDR geometry passes (matches `msaa_sample_count`).
    depth_msaa_texture: wgpu::Texture,
    depth_msaa_view: wgpu::TextureView,
    /// Previous frame depth (R32Float) for TAA disocclusion.
    prev_depth_texture: wgpu::Texture,
    prev_depth_view: wgpu::TextureView,

    /// View of the directional shadow map.
    pub shadow_view: wgpu::TextureView,
    /// Comparison sampler for shadow maps.
    pub shadow_sampler: wgpu::Sampler,
    /// Atlas of point-light shadow maps.
    pub local_shadow_atlas: wgpu::Texture,
    /// View of the local shadow atlas.
    pub local_shadow_atlas_view: wgpu::TextureView,

    // --- GPU resource registry ---
    /// GPU resources of meshes, materials and textures.
    pub registry: GpuRegistry,
    /// Bind group with fallback textures.
    pub fallback_texture_bg: wgpu::BindGroup,
    /// Fallback normal map (flat).
    pub fallback_normal_view: wgpu::TextureView,
    /// Fallback metallic-roughness texture.
    pub fallback_mr_view: wgpu::TextureView,
    /// Sampler shared by material textures.
    pub common_sampler: wgpu::Sampler,
    /// Per-material blend mode, parallel to registry legacy material indices.
    pub material_blend_flags: Vec<BlendMode>,

    /// Per-object model-matrix buffer (reused each frame).
    model_buffer: wgpu::Buffer,
    model_bind_group: wgpu::BindGroup,
    /// Bind group for the model buffer used by shadow passes.
    pub shadow_model_bind_group: wgpu::BindGroup,

    // --- Clustered Forward Rendering ---
    /// Assigns lights to view-space clusters.
    pub cluster_manager: ClusterManager,

    /// Forward lighting path (clustered vs ScatterLight).
    pub lighting_mode: LightingMode,
    /// ScatterLight fullscreen pass (used when [`LightingMode::ScatterLight`]).
    pub scatter_light_pass: ScatterLightPass,
    scatter_light_bind_group: wgpu::BindGroup,
    // --- Sharp AA (MSAA + Smart Resolve + SMAA + RCAS) ---
    /// When true, the Sharp AA pipeline is active: MSAA geometry → luma-weighted
    /// resolve → ACES tonemap → SMAA 1× → RCAS sharpening → surface.
    pub aa_sharp: bool,
    /// Luma-weighted MSAA resolve (replaces hardware `resolve_target`).
    msaa_resolve_pass: MsaaResolvePass,
    msaa_resolve_bg: wgpu::BindGroup,
    /// SMAA 1× post-process (edge → blend weights → neighbourhood).
    smaa_pass: SmaaPass,
    /// LDR intermediate after ACES tonemap (Rgba8Unorm, output resolution).
    ldr_texture: wgpu::Texture,
    ldr_view: wgpu::TextureView,
    /// SMAA output (Rgba8Unorm, output resolution).
    smaa_out_texture: wgpu::Texture,
    smaa_out_view: wgpu::TextureView,
    /// Tonemap pipeline targeting Rgba8Unorm (ACES+bloom, no RCAS).
    tone_mapping_ldr_pipeline: wgpu::RenderPipeline,
    /// Bind group: HDR+bloom → ldr_view.
    tone_mapping_ldr_bg: wgpu::BindGroup,
    /// RCAS-only present pipeline (surface format, no ACES).
    rcas_present_pipeline: wgpu::RenderPipeline,
    /// Bind group: SMAA output → surface with RCAS.
    tone_mapping_rcas_bg: wgpu::BindGroup,
    /// Dedicated uniform buffer for the RCAS present pass.
    tonemap_rcas_uniform_buffer: wgpu::Buffer,

    /// Dual-filter bloom post-process (6-mip chain).
    pub bloom_pass: BloomPass,
    /// Whether bloom is applied.
    pub bloom_enabled: bool,
    bloom_mip_textures: [wgpu::Texture; crate::post::bloom::BLOOM_MIP_COUNT],
    bloom_mip_views: [wgpu::TextureView; crate::post::bloom::BLOOM_MIP_COUNT],
    bloom_scratch_texture: wgpu::Texture,
    bloom_scratch_view: wgpu::TextureView,
    /// Downsample bind groups: [0]=HDR→mip0, [1]=mip0→mip1, …
    bloom_down_bgs: [wgpu::BindGroup; crate::post::bloom::BLOOM_MIP_COUNT],
    /// Upsample bind groups: [i] reads mip(i+1) into mip(i).
    bloom_up_bgs: [wgpu::BindGroup; crate::post::bloom::BLOOM_MIP_COUNT - 1],
    bloom_composite_bg: wgpu::BindGroup,

    // --- Phase 5 GI / SDF ---
    /// GPU side of the DDGI probe grid.
    pub ddgi_gpu: crate::gi::DdgiGpu,
    /// GPU signed distance field atlas.
    pub sdf_gpu: crate::sdf::SdfGpuAtlas,
    #[allow(dead_code)]
    blue_noise_texture: wgpu::Texture,
    blue_noise_view: wgpu::TextureView,
    blue_noise_sampler: wgpu::Sampler,
    /// Hi-Z depth pyramid for VG occlusion culling (previous frame).
    pub hiz: HiZPyramid,
    /// When true, mesh uploads use 16-byte compressed vertices (default).
    pub use_compressed_vertices: bool,

    // --- IBL ---
    /// Converts environment maps into IBL textures.
    pub ibl_processor: IBLProcessor,
    /// The current image-based lighting environment.
    pub ibl_resource: IBLResource,

    // --- Debug visualization (e.g. audio occlusion rays) ---
    /// Pass that draws debug lines.
    pub debug_lines_pass: DebugLinesPass,
    /// Lines to draw this frame: (start, end, occluded). Set via [`Self::set_debug_lines`].
    pub current_debug_lines: Vec<(Vec3, Vec3, bool)>,
    /// Camera-only bind group (for debug lines pipeline).
    camera_bind_group: wgpu::BindGroup,

    // --- Temporal Anti-Aliasing (TAA) ---
    /// Enable TAA.  When `false` the velocity pass and TAA pass are skipped
    /// and tone mapping reads directly from the HDR buffer (no change to
    /// existing behaviour).
    pub taa_enabled: bool,
    /// Number of frames rendered since the last reset (drives jitter sequence).
    pub taa_frame_count: u64,
    /// Previous frame's **unjittered** view-projection matrix.
    /// Written at the end of each frame; read by the velocity pass at the start
    /// of the next.
    pub taa_prev_vp_unjittered: Mat4,
    /// TAA accumulation pass (neighbour AABB clamp + blend).
    pub taa_pass: TaaPass,
    /// Velocity (motion-vector) generation pass.
    pub velocity_pass: VelocityPass,
    /// Redraws moving objects into the velocity texture.
    pub object_velocity_pass: ObjectVelocityPass,
    /// Previous-frame model matrices (parallel to `model_buffer`).
    prev_model_buffer: wgpu::Buffer,
    object_velocity_bg: wgpu::BindGroup,
    /// Which objects moved and where they were last frame.
    object_motion: motion::ObjectMotion,
    /// Object ids supplied for the frame being rendered (see `FrameView::with_object_ids`).
    frame_object_ids: Option<Vec<u64>>,
    /// Ping-pong history texture 0.
    pub taa_history_tex: [wgpu::Texture; 2],
    /// Views into the ping-pong history textures.
    pub taa_history_view: [wgpu::TextureView; 2],
    /// Velocity (motion vector) texture.
    pub taa_velocity_tex: wgpu::Texture,
    /// View into the velocity texture.
    pub taa_velocity_view: wgpu::TextureView,
    /// Index of the history texture that was written **last frame** (the read
    /// source for the next frame).  Flipped at the end of each TAA frame.
    pub taa_history_read_idx: usize,
    /// TAA blend factor: weight of the *current* frame in accumulation.
    /// Lower reduces aliasing/shimmer, but can increase ghosting.
    pub taa_blend_alpha: f32,
    /// Velocity pass bind group (samples the depth buffer). Rebuilt on resize.
    pub velocity_bg: wgpu::BindGroup,
    /// Prebuilt TAA accumulation bind groups, indexed by history read idx.
    pub taa_history_bgs: [wgpu::BindGroup; 2],
    /// Prebuilt tone-mapping bind groups over each history slot, indexed by
    /// the history write idx of the current frame.
    pub taa_tone_bgs: [wgpu::BindGroup; 2],
    // --- Skybox (background env pass; None until set_skybox(true)) ---
    /// Skybox pass; present once a real environment is set.
    pub skybox_pass: Option<SkyboxPass>,

    // --- Transparent geometry (alpha-blended PBR variant) ---
    /// Transparent object pass.
    pub transparent_pass: TransparentPass,

    // --- Virtual Shadow Maps (directional; enabled via `set_shadow_mode`) ---
    /// Pass that renders depth into virtual shadow map pages.
    pub vsm_depth_pass: crate::shadow::depth_pass::VsmDepthPass,
    /// Clipmap of virtual shadow map pages.
    pub vsm_clipmap: crate::shadow::vsm_clipmap::VsmClipmap,
    /// Per-page R32Float layers (one layer per physical page).
    pub vsm_pages_tex: wgpu::Texture,
    /// Views of the physical shadow pages.
    pub vsm_pages_views: Vec<wgpu::TextureView>,
    /// Dynamic-offset uniform feeding `PageDrawInfo` per rendered page.
    pub vsm_pageinfo_buffer: wgpu::Buffer,
    /// Bind group with the page table information.
    pub vsm_pageinfo_bg: wgpu::BindGroup,
    /// Group-25 resources sampled by the PBR shader in VSM mode.
    pub vsm_info_buffer: wgpu::Buffer,
    /// Sampler for the virtual shadow maps.
    pub vsm_sampler: wgpu::Sampler,
    /// Runtime switch for VSM + SMRT sampling.
    pub vsm_enabled: bool,
    /// Per-pass GPU timings (needs timestamp-query features on the device).
    pub gpu_profiler: crate::profiler::GpuProfiler,
    /// Multiplier for image-based (environment) lighting; 1.0 = as authored.
    pub ibl_intensity: f32,
    /// GPU feedback: which VSM pages the visible pixels need.
    vsm_page_request: crate::shadow::page_request::PageRequestPass,
    /// Frames rendered with VSM on (the first few block on the feedback readback).
    vsm_request_frames: u32,
    /// Screen-space VSM trace (1 Vogel ray + temporal, 90-95% history) — default ON.
    /// PBR samples `t_vsm_shadow` (1 filtered fetch) instead of inline 16-ray `vsm_shadow()`.
    pub vsm_screen_shadows: bool,
    /// Number of shadow pages rendered last frame.
    pub vsm_dirty_pages_last_frame: u32,
    /// SMRT ray/sample budget uploaded with VSM info.
    pub smrt_config: crate::shadow::smrt::SmrtConfig,
    /// Frames since last VSM shadow temporal reset (independent of TAA counter).
    vsm_shadow_frame_count: u64,
    /// Temporal blend alpha for VSM screen shadows (weight of current frame, history = 1-alpha).
    vsm_shadow_temporal_alpha: f32,
    /// True after at least one VSM frame with zero dirty pages (clipmap cached).
    vsm_warm: bool,
    /// Debounce for dynamic invalidation: centers from previous frame, only invalidate if moved >0.5m.
    prev_dynamic_centers: Vec<redox_math::Vec3>,
    /// Cached D2Array view for VSM pages (avoid per-frame view alloc).
    vsm_pages_array_view: wgpu::TextureView,
    vg_bind_groups_dirty: bool,

    // --- Virtual Geometry (Nanite-like) ---
    /// Virtual geometry system, when enabled.
    pub vg_system: Option<VGSystem>,
    /// Visibility-buffer raster pass (Phase 3).
    pub vg_vis_pass: VgVisPass,
    /// Fullscreen PBR resolve from visibility buffer.
    pub vg_resolve_pass: VgResolvePass,
    /// Depth-only meshlet pass for shadow maps.
    pub vg_shadow_pass: VgShadowPass,
    /// Render targets of the visibility buffer.
    pub vg_vis_targets: VgVisTargets,
    vg_material_buffer: wgpu::Buffer,
    vg_resolve_bind_group: wgpu::BindGroup,
    vg_shadow_instances_bg: wgpu::BindGroup,
    /// When true, VG uses visibility buffer + PBR resolve instead of debug forward path.
    pub vg_use_vis_path: bool,

    /// Point lights for the current frame (shared between update_cluster_lights and render_frame_into).
    pub current_point_lights: Vec<crate::light::PointLight>,

    /// Reused each frame — avoids `Vec<ModelUniform>` allocation in upload.
    model_uniform_scratch: Vec<ModelUniform>,
    /// Reused each frame — avoids `Vec<bool>` allocation for main-view cull.
    main_vis_scratch: Vec<bool>,
    /// Reused while syncing VG entity transforms from ECS.
    vg_sync_scratch: Vec<(
        redox_ecs::Entity,
        [[f32; 4]; 4],
        Option<crate::virtual_geometry::VGInstanceHandle>,
        VGAssetId,
    )>,

    /// Per-frame aligned upload ring (model / cluster / VSM / uniforms).
    pub staging_ring: crate::resource::StagingRing,

    // ── Per-frame reusable shadow resources (allocated once, written each frame) ──
    /// Cube-face matrix buffers for point-light shadows (6 faces).
    pub face_matrix_buffers: Vec<wgpu::Buffer>,
    /// Bind groups pointing at [`Self::face_matrix_buffers`].
    pub face_matrix_bgs: Vec<wgpu::BindGroup>,

    /// Optional GPU particle billboards (Phase 4.1).
    pub particle_system: Option<ParticleGpuSystem>,
    particle_sim_uniforms: ParticleSimUniforms,
    /// R8 reactive mask for TAA (written by particles / transparent FX).
    reactive_texture: Option<wgpu::Texture>,
    reactive_view: Option<wgpu::TextureView>,
    /// When true, the normal prepass records on a worker thread.
    pub mt_command_recording: bool,
}

impl Drop for Renderer {
    /// Persists the pipeline cache so the next start skips shader compilation.
    fn drop(&mut self) {
        if let Some(bytes) = crate::pipeline_cache::save() {
            log::info!("pipeline cache saved ({bytes} bytes)");
        }
    }
}
