//! Core renderer: owns wgpu device/queue/surface, passes, and GPU registry.

mod contexts;

use bytemuck;
use std::sync::Arc;
use winit::window::Window;

use rayon::join;
use redox_asset::Handle;
use redox_vfx::{ParticleSimUniforms, ParticleSystem};

use contexts::{
    compute_main_visibility, record_csm_shadow_depth, record_normal_prepass,
    record_point_atlas_shadows, record_vsm_pages, AtlasCasterJob, AtlasContext, PrepassContext,
    ShadowContext, VsmContext,
};

use crate::asset_types::{BlendMode, MaterialData, MeshData, TextureData};
use crate::camera::CameraUniform;
use crate::cluster_manager::ClusterManager;
use crate::light::{LightUniform, LightingMode, ShaderDebugUniform};

/// Debug visualization mode for the PBR shader (binding 21).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum ShaderDebugVizMode {
    #[default]
    None = 0,
    ClusterIndex = 1,
    ShadowValue = 2,
    /// Magenta=SSAO · Cyan=CSM · Lime=VSM · Yellow=SDF (darkening amounts).
    ShadowSources = 3,
}
use crate::material::Material;
use crate::mesh::{Mesh, compress_vertices, bounds_vectors};
use crate::pass::debug_lines::DebugLinesPass;
use crate::pass::shared_layouts::{PbrFrameLayouts, SharedLayouts, ModelUniform, create_depth_texture};
use crate::pass::particles::ParticleGpuSystem;
use crate::pass::scatter_light::{ScatterLightPass, ScatterLightUniform};
use crate::pass::normal::NormalPass;
use crate::pass::pbr::PbrPass;
use crate::pass::shadow::{SHADOW_FORMAT, SHADOW_SIZE, ShadowPass};
use crate::pass::skybox::SkyboxPass;
use crate::pass::transparent::TransparentPass;
use crate::pass::vg_resolve::{
    build_material_storage, create_vg_vis_targets, VgResolvePass, VgShadowPass, VgVisPass,
    VgVisTargets, VIS_ID_SENTINEL,
};
use crate::shadow::csm::{CsmConfig, CsmState, ShadowUniform};
use crate::virtual_geometry::hiz::HiZPyramid;
use crate::virtual_geometry::runtime::{VGConfig, VGSystem};
use crate::systems::Transform;
use crate::virtual_geometry::{VirtualMesh, VGAssetId};
use crate::pass::taa::{
    TaaPass, TaaUniform,
    apply_jitter_to_projection, create_history_texture, create_velocity_texture,
    halton_jitter_ndc,
};
use crate::pass::velocity::{VelocityPass, VelocityUniform};
use redox_math::{Aabb, Mat3, Mat4, Vec3};
use crate::post::ssao::{SSAOPass, VsmPassUniform, VsmShadowPass, VsmTemporalUniform};
use crate::post::bloom::BloomPass;
use crate::pass::msaa_resolve::MsaaResolvePass;
use crate::pass::smaa::SmaaPass;
use crate::post::tone_mapping::{
    TonemapUniform, create_tone_mapping_pipeline, create_tonemap_bind_group,
    create_rcas_present_pipeline,
};
use crate::resource::buffer;
use crate::resource::ibl::{IBLProcessor, IBLResource};
use crate::resource::texture::Texture;
use crate::systems::RenderObject;
use crate::camera::Camera;
use crate::frame::FrameView;
use crate::registry::{GpuMesh, GpuRegistry};

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
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// Window swapchain. `None` for offscreen editor hosts that only call
    /// [`Self::render_frame_into`] (egui SharedGpu owns the real surface).
    pub surface: Option<wgpu::Surface<'static>>,
    pub config: wgpu::SurfaceConfiguration,
    /// Internal render scale for TAAU (render at lower res, upscale temporally).
    pub render_scale: f32,
    /// Internal render width in pixels.
    pub internal_width: u32,
    /// Internal render height in pixels.
    pub internal_height: u32,

    // --- Render pass ---
    pub shared_layouts: SharedLayouts,
    /// Planned decoupled PBR frame bind groups (Phase 2.2 factories).
    pub pbr_frame_layouts: PbrFrameLayouts,
    pub pbr_pass: PbrPass,
    pub shadow_pass: ShadowPass,
    pub normal_pass: NormalPass,
    pub ssao_pass: SSAOPass,

    // --- Global Uniforms ---
    pub camera_buffer: wgpu::Buffer,
    pub camera_uniform: CameraUniform,
    /// Near/far of the last uploaded camera — must match `camera_uniform.proj`
    /// when building CSM cascades (mismatched far → detached shadow chunks).
    pub camera_near: f32,
    pub camera_far: f32,

    pub light_uniform: LightUniform,
    pub light_buffer: wgpu::Buffer,
    pub shader_debug_buffer: wgpu::Buffer,

    pub frame_bind_group: wgpu::BindGroup,
    pub scene_bind_group: wgpu::BindGroup,

    // --- HDR & Post-processing ---
    /// Multisample count for HDR geometry (2× when TAA is enabled).
    pub msaa_sample_count: u32,
    /// Preferred HDR MSAA count when TAA is on (queried from adapter at init).
    hdr_msaa_target_count: u32,
    pub hdr_texture: wgpu::Texture,
    pub hdr_view: wgpu::TextureView,
    /// 2× multisampled HDR colour target for geometry passes.
    pub hdr_msaa_texture: wgpu::Texture,
    pub hdr_msaa_view: wgpu::TextureView,
    pub tone_mapping_pipeline: wgpu::RenderPipeline,
    pub tone_mapping_bind_group: wgpu::BindGroup,
    /// Uniform for the tone-mapping pass (`TonemapUniform`: exposure /
    /// sharpen toggle / output size). Written each frame before submission.
    pub tonemap_uniform_buffer: wgpu::Buffer,
    /// Linear exposure multiplier fed to the ACES tone mapper (default 1.0).
    pub exposure: f32,
    /// RCAS sharpness in stops (lower = sharper post-TAA recovery).
    pub rcas_sharpness: f32,
    pub hdr_sampler: wgpu::Sampler,

    // --- Normal pass + SSAO ---
    pub normal_texture: wgpu::Texture,
    pub normal_view: wgpu::TextureView,
    pub normal_bind_group: wgpu::BindGroup,
    pub ssao_raw_texture: wgpu::Texture,
    pub ssao_raw_view: wgpu::TextureView,
    /// Half-res bilateral blur output (fed into the upsample pass).
    pub ssao_half_blurred_texture: wgpu::Texture,
    pub ssao_half_blurred_view: wgpu::TextureView,
    pub ssao_blurred_texture: wgpu::Texture,
    pub ssao_blurred_view: wgpu::TextureView,
    pub ssao_bind_group: wgpu::BindGroup,
    pub ssao_blur_bind_group: wgpu::BindGroup,
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
    #[allow(dead_code)]
    pub depth_texture: wgpu::Texture,
    pub depth_view: wgpu::TextureView,
    /// Multisampled depth for HDR geometry passes (matches `msaa_sample_count`).
    depth_msaa_texture: wgpu::Texture,
    depth_msaa_view: wgpu::TextureView,
    /// Previous frame depth (R32Float) for TAA disocclusion.
    prev_depth_texture: wgpu::Texture,
    prev_depth_view: wgpu::TextureView,

    pub shadow_view: wgpu::TextureView,
    pub shadow_sampler: wgpu::Sampler,
    pub csm_texture: wgpu::Texture,
    pub csm_view: wgpu::TextureView,
    pub csm_sampler: wgpu::Sampler,
    pub local_shadow_atlas: wgpu::Texture,
    pub local_shadow_atlas_view: wgpu::TextureView,
    pub shadow_uniform: ShadowUniform,
    pub shadow_uniform_buffer: wgpu::Buffer,

    // --- GPU resource registry ---
    pub registry: GpuRegistry,
    pub fallback_texture_bg: wgpu::BindGroup,
    pub fallback_normal_view: wgpu::TextureView,
    pub fallback_mr_view: wgpu::TextureView,
    pub common_sampler: wgpu::Sampler,
    /// Per-material blend mode, parallel to registry legacy material indices.
    pub material_blend_flags: Vec<BlendMode>,

    /// Per-object model-matrix buffer (reused each frame).
    model_buffer: wgpu::Buffer,
    model_bind_group: wgpu::BindGroup,
    pub shadow_model_bind_group: wgpu::BindGroup,

    // --- Clustered Forward Rendering ---
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
    pub ddgi_gpu: crate::gi::DdgiGpu,
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
    pub ibl_processor: IBLProcessor,
    pub ibl_resource: IBLResource,

    // --- Debug visualization (e.g. audio occlusion rays) ---
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
    pub skybox_pass: Option<SkyboxPass>,

    // --- Transparent geometry (alpha-blended PBR variant) ---
    pub transparent_pass: TransparentPass,

    // --- Virtual Shadow Maps (directional; enabled via `set_shadow_mode`) ---
    pub vsm_depth_pass: crate::shadow::depth_pass::VsmDepthPass,
    pub vsm_clipmap: crate::shadow::vsm_clipmap::VsmClipmap,
    /// Per-page R32Float layers (one layer per physical page).
    pub vsm_pages_tex: wgpu::Texture,
    pub vsm_pages_views: Vec<wgpu::TextureView>,
    /// Dynamic-offset uniform feeding `PageDrawInfo` per rendered page.
    pub vsm_pageinfo_buffer: wgpu::Buffer,
    pub vsm_pageinfo_bg: wgpu::BindGroup,
    /// Group-25 resources sampled by the PBR shader in VSM mode.
    pub vsm_info_buffer: wgpu::Buffer,
    pub vsm_sampler: wgpu::Sampler,
    /// Runtime switch: false = classic CSM path, true = VSM + SMRT sampling.
    pub vsm_enabled: bool,
    /// Screen-space VSM trace (1 Vogel ray + temporal, 90-95% history) — default ON.
    /// PBR samples `t_vsm_shadow` (1 filtered fetch) instead of inline 16-ray `vsm_shadow()`.
    pub vsm_screen_shadows: bool,
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

    // --- CSM state ---
    pub csm_state: CsmState,

    // --- Virtual Geometry (Nanite-like) ---
    pub vg_system: Option<VGSystem>,
    /// Visibility-buffer raster pass (Phase 3).
    pub vg_vis_pass: VgVisPass,
    /// Fullscreen PBR resolve from visibility buffer.
    pub vg_resolve_pass: VgResolvePass,
    /// Depth-only meshlet pass for shadow maps.
    pub vg_shadow_pass: VgShadowPass,
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
    /// Cascade matrix uniform buffers, one per CSM cascade slot.
    pub cascade_matrix_buffers: Vec<wgpu::Buffer>,
    /// Bind groups pointing at [`Self::cascade_matrix_buffers`].
    pub cascade_matrix_bgs: Vec<wgpu::BindGroup>,
    /// Cached per-layer views into the CSM texture array.
    pub csm_layer_views: Vec<wgpu::TextureView>,
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
    /// When true, CSM shadow + normal prepass record on worker threads.
    pub mt_command_recording: bool,
}

impl Renderer {
    /// Initialises the entire rendering context.
    ///
    /// This is an `async` function because `wgpu` adapter and device requests
    /// are asynchronous. Wrap with `pollster::block_on` when calling from
    /// synchronous code.
    pub async fn new(window: Arc<Window>, present_mode: wgpu::PresentMode) -> Self {
        let size = window.inner_size();

        // --- Instance ---
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        // --- Surface ---
        let surface = instance
            .create_surface(window)
            .expect("Failed to create surface");

        // --- Adapter ---
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("Failed to find a suitable GPU adapter");

        log::info!("Using adapter: {:?}", adapter.get_info().name);

        // --- Device & Queue ---
        let mut required_features = wgpu::Features::empty();
        if adapter.features().contains(wgpu::Features::SHADER_PRIMITIVE_INDEX) {
            required_features |= wgpu::Features::SHADER_PRIMITIVE_INDEX;
        }
        // Rgba16Float MSAA×2 needs adapter-specific format features (WebGPU baseline is ×1/×4 only).
        if adapter
            .features()
            .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
        {
            required_features |= wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
        }
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("redox_device"),
                    required_features,
                    required_limits: wgpu::Limits::default(),
                },
                None,
            )
            .await
            .expect("Failed to create device");

        // --- Surface configuration ---
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        Self::from_shared_device(device, queue, &adapter, Some(surface), config)
    }

    /// Build a full engine [`Renderer`] on an existing device/queue (editor SharedGpu).
    ///
    /// Pass `surface: None` for offscreen hosts that only call [`Self::render_frame_into`].
    /// `config.format` must match the `surface_view` passed to that method (editor RTT =
    /// [`wgpu::TextureFormat::Rgba8Unorm`]).
    pub fn from_shared_device(
        device: wgpu::Device,
        queue: wgpu::Queue,
        adapter: &wgpu::Adapter,
        surface: Option<wgpu::Surface<'static>>,
        config: wgpu::SurfaceConfiguration,
    ) -> Self {
        let staging_ring = crate::resource::StagingRing::new(
            &device,
            crate::resource::DEFAULT_RING_CAPACITY,
        );

        if let Some(ref surface) = surface {
            surface.configure(&device, &config);
        }

        let surface_format = config.format;
        let hdr_format = wgpu::TextureFormat::Rgba16Float;

        // --- Internal resolution for TAAU ---
        // Default quality: render above output resolution, then TAA resolve.
        // This behaves like SSAA/temporal supersampling and reduces stair-steps without adding blur.
        let render_scale = 1.0_f32;
        let internal_width = ((config.width as f32) * render_scale).round().max(1.0) as u32;
        let internal_height = ((config.height as f32) * render_scale).round().max(1.0) as u32;
        // Weight of the current frame in TAA (higher = sharper, lower = smoother).
        let taa_blend_alpha = 0.08_f32;
        let taa_enabled = false;
        let aa_sharp = true;
        let hdr_msaa_target_count = pick_hdr_msaa_sample_count(&adapter, hdr_format, aa_sharp || taa_enabled);
        let msaa_sample_count = hdr_msaa_target_count;
        log::info!("HDR MSAA sample count: {msaa_sample_count} (TAA enabled: {taa_enabled})");

        // --- Render passes ---
        let shared_layouts = SharedLayouts::new(&device);
        let pbr_frame_layouts = PbrFrameLayouts::new(&device);
        let pbr_pass = PbrPass::new(&device, hdr_format, msaa_sample_count);
        let transparent_pass =
            TransparentPass::new(&device, &pbr_pass, hdr_format, msaa_sample_count);
        let shadow_pass = ShadowPass::new(&device);
        let normal_pass = NormalPass::new(&device, wgpu::TextureFormat::Rgba16Float);
        let ssao_pass = SSAOPass::new(&device, &queue);
        let vsm_shadow_pass = VsmShadowPass::new(&device);
        let scatter_light_pass = ScatterLightPass::new(&device, hdr_format);
        let bloom_pass = BloomPass::new(&device, hdr_format);
        let debug_lines_pass =
            DebugLinesPass::new(&device, &shared_layouts.camera_bgl, msaa_sample_count);

        // --- Camera uniform ---
        let camera_uniform = CameraUniform::default();
        let camera_buffer = buffer::create_uniform_buffer(
            &device,
            "camera_uniform",
            bytemuck::bytes_of(&camera_uniform),
        );
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera_bind_group"),
            layout: &shared_layouts.camera_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        // --- Shadow Map / CSM depth array ---
        let shadow_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow_texture"),
            size: wgpu::Extent3d {
                width: SHADOW_SIZE,
                height: SHADOW_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_view = shadow_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow_comparison_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });

        // CSM texture array (4 cascades).
        let csm_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("csm_texture_array"),
            size: wgpu::Extent3d {
                width: SHADOW_SIZE,
                height: SHADOW_SIZE,
                depth_or_array_layers: 4,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let csm_view = csm_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("csm_texture_array_view"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        let csm_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("csm_comparison_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });

        // Local lights static shadow atlas.
        let local_shadow_atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("local_shadow_atlas"),
            size: wgpu::Extent3d {
                width: 8192,
                height: 8192,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let local_shadow_atlas_view =
            local_shadow_atlas.create_view(&wgpu::TextureViewDescriptor::default());

        let shadow_uniform = ShadowUniform::default();
        let shadow_uniform_buffer = buffer::create_uniform_buffer(
            &device,
            "shadow_uniform",
            bytemuck::bytes_of(&shadow_uniform),
        );

        // --- Light uniform ---
        let light_uniform = LightUniform::default();
        let light_buffer = buffer::create_uniform_buffer(
            &device,
            "light_uniform",
            bytemuck::bytes_of(&light_uniform),
        );

        let shader_debug_uniform = ShaderDebugUniform::default();
        let shader_debug_buffer = buffer::create_uniform_buffer(
            &device,
            "shader_debug_uniform",
            bytemuck::bytes_of(&shader_debug_uniform),
        );

        // --- Depth (internal) ---
        let (depth_texture, depth_view) =
            create_depth_texture(&device, internal_width, internal_height);
        let (prev_depth_texture, prev_depth_view) =
            create_prev_depth_texture(&device, &queue, internal_width, internal_height);

        // --- HDR Texture (internal) ---
        let hdr_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hdr_texture"),
            size: wgpu::Extent3d {
                width: internal_width,
                height: internal_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: hdr_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let hdr_view = hdr_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let (hdr_msaa_texture, hdr_msaa_view) = create_hdr_msaa_texture(
            &device,
            internal_width,
            internal_height,
            msaa_sample_count,
        );
        let (depth_msaa_texture, depth_msaa_view) = create_depth_msaa_texture(
            &device,
            internal_width,
            internal_height,
            msaa_sample_count,
        );

        let hdr_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("hdr_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let (bloom_mip_textures, bloom_mip_views) =
            create_bloom_mip_chain(&device, internal_width, internal_height, hdr_format);
        let (bloom_scratch_texture, bloom_scratch_view) = create_bloom_texture(
            &device,
            internal_width,
            internal_height,
            hdr_format,
            "bloom_scratch",
        );
        let bloom_down_bgs = create_bloom_downsample_bgs(
            &bloom_pass,
            &device,
            &hdr_view,
            &bloom_mip_views,
            &hdr_sampler,
        );
        let bloom_up_bgs =
            create_bloom_upsample_bgs(&bloom_pass, &device, &bloom_mip_views, &hdr_sampler);
        let bloom_composite_bg = bloom_pass.create_composite_bind_group(
            &device,
            &hdr_view,
            &bloom_mip_views[0],
            &hdr_sampler,
        );

        let tone_mapping_pipeline = create_tone_mapping_pipeline(&device, surface_format);
        // Tone-mapping uniform (exposure / sharpen toggle / output size).
        // The buffer is size-independent: `output_size` is written per frame.
        let tonemap_uniform_buffer = buffer::create_uniform_buffer(
            &device,
            "tonemap_uniform",
            &[0u8; std::mem::size_of::<TonemapUniform>()],
        );
        let tone_mapping_bind_group = create_tonemap_bind_group(
            &device,
            &tone_mapping_pipeline,
            &hdr_view,
            &bloom_mip_views[0],
            &hdr_sampler,
            &tonemap_uniform_buffer,
        );

        // --- Sharp AA resources ---
        let msaa_resolve_pass = MsaaResolvePass::new(&device, hdr_format);
        msaa_resolve_pass.set_sample_count(&queue, msaa_sample_count);
        let msaa_resolve_bg = msaa_resolve_pass.create_bind_group(&device, &hdr_msaa_view);

        let mut smaa_pass = SmaaPass::new(&device, &queue);
        smaa_pass.resize(&device, config.width, config.height);

        let (ldr_texture, ldr_view) = create_sharp_intermediate(
            &device, config.width, config.height, "sharp_ldr",
        );
        let (smaa_out_texture, smaa_out_view) = create_sharp_intermediate(
            &device, config.width, config.height, "sharp_smaa_out",
        );

        let tone_mapping_ldr_pipeline =
            create_tone_mapping_pipeline(&device, wgpu::TextureFormat::Rgba8Unorm);
        let tone_mapping_ldr_bg = create_tonemap_bind_group(
            &device,
            &tone_mapping_ldr_pipeline,
            &hdr_view,
            &bloom_mip_views[0],
            &hdr_sampler,
            &tonemap_uniform_buffer,
        );
        let rcas_present_pipeline = create_rcas_present_pipeline(&device, surface_format);
        let tonemap_rcas_uniform_buffer = buffer::create_uniform_buffer(
            &device,
            "tonemap_rcas_uniform",
            &[0u8; std::mem::size_of::<TonemapUniform>()],
        );
        let tone_mapping_rcas_bg = create_tonemap_bind_group(
            &device,
            &rcas_present_pipeline,
            &smaa_out_view,
            &bloom_mip_views[0],
            &hdr_sampler,
            &tonemap_rcas_uniform_buffer,
        );

        // --- Fallback textures ---
        let fallback_tex = Texture::white_1x1(&device, &queue);
        let fallback_texture_bg = shared_layouts.create_texture_bind_group(&device, &fallback_tex);

        let fallback_normal = Texture::normal_1x1(&device, &queue);
        let fallback_mr = Texture::mr_1x1(&device, &queue);
        let common_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("common_material_sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let mut registry = GpuRegistry::new();
        registry.push_legacy_texture(fallback_tex, fallback_texture_bg);
        let fallback_texture_bg = shared_layouts.create_texture_bind_group(
            &device,
            registry.legacy_texture(0).expect("fallback texture"),
        );

        // --- Per-object model buffer (Storage) ---
        let model_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("model_storage_buffer"),
            size: (std::mem::size_of::<ModelUniform>() * 10000) as u64, // Support up to 10,000 objects
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let model_bind_group = shared_layouts.create_model_bind_group(&device, &model_buffer);
        let shadow_model_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow_model_bg"),
            layout: &shadow_pass.model_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: model_buffer.as_entire_binding(),
            }],
        });

        let half_width = (internal_width / 2).max(1);
        let half_height = (internal_height / 2).max(1);

        // --- Normal + GTAO textures (internal) ---
        let gtao_fmt = wgpu::TextureFormat::Rgba16Float;
        let (normal_texture, normal_view) = create_normal_ssao_texture(
            &device,
            internal_width,
            internal_height,
            wgpu::TextureFormat::Rgba16Float,
            "normal",
        );
        let (ssao_raw_texture, ssao_raw_view) = create_normal_ssao_texture(
            &device,
            half_width,
            half_height,
            gtao_fmt,
            "gtao_raw",
        );
        let (ssao_half_blurred_texture, ssao_half_blurred_view) = create_normal_ssao_texture(
            &device,
            half_width,
            half_height,
            gtao_fmt,
            "gtao_half_blurred",
        );
        let (ssao_blurred_texture, ssao_blurred_view) = create_normal_ssao_texture(
            &device,
            internal_width,
            internal_height,
            gtao_fmt,
            "gtao_blurred",
        );

        let (vsm_shadow_raw_texture, vsm_shadow_raw_view) = create_normal_ssao_texture(
            &device,
            internal_width,
            internal_height,
            wgpu::TextureFormat::R16Float,
            "vsm_shadow_raw",
        );
        let (vsm_shadow_filtered_texture, vsm_shadow_filtered_view) = create_normal_ssao_texture(
            &device,
            internal_width,
            internal_height,
            wgpu::TextureFormat::R16Float,
            "vsm_shadow_filtered",
        );
        let (vsm_shadow_history_texture, vsm_shadow_history_view) = create_normal_ssao_texture(
            &device,
            internal_width,
            internal_height,
            wgpu::TextureFormat::R16Float,
            "vsm_shadow_history",
        );

        let ssao_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ssao_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let ssao_pbr_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gtao_pbr_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let vsm_shadow_pbr_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("vsm_shadow_pbr_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let normal_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("normal_bind_group"),
            layout: &normal_pass.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: model_buffer.as_entire_binding(),
                },
            ],
        });

        let ssao_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gtao_bind_group"),
            layout: &ssao_pass.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&normal_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&ssao_pass.noise_texture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&ssao_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: camera_buffer.as_entire_binding(),
                },
            ],
        });

        let ssao_blur_bind_group = ssao_pass.create_blur_bind_group(
            &device,
            &ssao_raw_view,
            &common_sampler,
            &normal_view,
        );
        let ssao_upsample_bind_group = ssao_pass.create_upsample_bind_group(
            &device,
            &ssao_half_blurred_view,
            &common_sampler,
            &normal_view,
        );

        // --- TAA infrastructure ---
        let taa_pass = TaaPass::new(&device, &queue, hdr_format);
        let velocity_pass = VelocityPass::new(&device);

        let (taa_history_tex0, taa_history_view0) =
            create_history_texture(&device, config.width, config.height, "taa_history_0");
        let (taa_history_tex1, taa_history_view1) =
            create_history_texture(&device, config.width, config.height, "taa_history_1");
        let (taa_velocity_tex, taa_velocity_view) =
            create_velocity_texture(&device, internal_width, internal_height);

        // Cached per-frame bind groups (rebuilt only on resize, not per frame).
        let velocity_bg = velocity_pass.create_bind_group(&device, &depth_view);
        let taa_history_bgs: [wgpu::BindGroup; 2] = [
            taa_pass.create_bind_group(
                &device,
                &hdr_view,
                &taa_velocity_view,
                &taa_history_view0,
                &taa_pass.reactive_fallback_view,
                &prev_depth_view,
            ),
            taa_pass.create_bind_group(
                &device,
                &hdr_view,
                &taa_velocity_view,
                &taa_history_view1,
                &taa_pass.reactive_fallback_view,
                &prev_depth_view,
            ),
        ];
        let make_tone_bg = |view: &wgpu::TextureView| {
            create_tonemap_bind_group(
                &device,
                &tone_mapping_pipeline,
                view,
                &bloom_mip_views[0],
                &hdr_sampler,
                &tonemap_uniform_buffer,
            )
        };
        let taa_tone_bgs: [wgpu::BindGroup; 2] =
            [make_tone_bg(&taa_history_view0), make_tone_bg(&taa_history_view1)];

        // --- IBL ---
        let ibl_processor = IBLProcessor::new(&device);
        let ibl_resource = IBLResource::dummy(&device, &queue);

        // --- Clustered Forward Rendering ---
        let dummy_camera = Camera::new(std::f32::consts::PI / 4.0, 16.0 / 9.0, 0.1, 1000.0);
        let cluster_manager = ClusterManager::new(internal_width, internal_height, &dummy_camera, &device, &queue);

        let scatter_light_bind_group = scatter_light_pass.create_bind_group(
            &device,
            &hdr_view,
            &depth_view,
            &hdr_sampler,
            &cluster_manager.point_lights_buffer,
        );

        let csm_state = CsmState::new(CsmConfig {
            cascade_count: 4,
            shadow_map_resolution: SHADOW_SIZE,
            near: 0.1,
            far: 200.0,
        });

        // ── VSM resources (directional; activated via `set_shadow_mode`) ──
        let vsm_config = crate::shadow::vsm_clipmap::VsmConfig {
            // 256² pages × 512 layers ≈ 128 MB (same budget as 128²×2048).
            pool_pages: 512.min(device.limits().max_texture_array_layers),
            ..Default::default()
        };
        let page_px = vsm_config.page_px;
        let layer_count = vsm_config.pool_pages;
        let vsm_pages_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vsm_pages"),
            size: wgpu::Extent3d {
                width: page_px,
                height: page_px,
                depth_or_array_layers: layer_count,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::shadow::shadow_atlas::VSM_ATLAS_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let vsm_pages_views: Vec<wgpu::TextureView> = (0..layer_count)
            .map(|l| {
                vsm_pages_tex.create_view(&wgpu::TextureViewDescriptor {
                    label: Some(&format!("vsm_page{l}")),
                    base_array_layer: l,
                    array_layer_count: Some(1),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    ..Default::default()
                })
            })
            .collect();
        let vsm_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("vsm_sampler"),
            ..Default::default()
        });
        let vsm_clipmap =
            crate::shadow::vsm_clipmap::VsmClipmap::new(crate::shadow::vsm_clipmap::VsmConfig {
                pool_pages: 512.min(device.limits().max_texture_array_layers),
                ..Default::default()
            });
        let info_size = std::mem::size_of::<crate::shadow::vsm_clipmap::VsmGpuInfo>() as u64;
        let vsm_info_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vsm_gpu_info"),
            size: info_size.max(16),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Phase 5: DDGI probe grid. SDF atlas starts empty — demo bricks must NOT
        // be uploaded globally (they multiply into directional shadows and look like
        // painted sliding blobs that don't match the real scene casters).
        let ddgi_gpu = crate::gi::DdgiGpu::new_default(&device);
        let sdf_gpu = crate::sdf::SdfGpuAtlas::new(&device, &queue);
        let (blue_noise_texture, blue_noise_view, blue_noise_sampler) =
            crate::resource::blue_noise::create_blue_noise_texture(&device, &queue);
        let hiz = HiZPyramid::new(&device, internal_width, internal_height);

        let vsm_pages_array_view = vsm_pages_tex.create_view(&wgpu::TextureViewDescriptor {
            label: Some("vsm_pages_whole"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        let vsm_shadow_trace_bind_group = vsm_shadow_pass.create_trace_bind_group(
            &device,
            &normal_view,
            &camera_buffer,
            &light_buffer,
            &vsm_pages_array_view,
            &vsm_sampler,
            &vsm_info_buffer,
            &blue_noise_view,
            &blue_noise_sampler,
        );
        let vsm_shadow_temporal_bind_group = vsm_shadow_pass.create_temporal_bind_group(
            &device,
            &vsm_shadow_raw_view,
            &vsm_shadow_history_view,
            &normal_view,
            &camera_buffer,
            &common_sampler,
        );

        let frame_bind_group = pbr_pass.create_frame_bind_group(
            &device,
            &camera_buffer,
            &shader_debug_buffer,
            &ibl_resource.irradiance_view,
            &ibl_resource.prefiltered_view,
            &ibl_resource.brdf_lut_view,
            &ibl_processor.sampler,
            &blue_noise_view,
            &blue_noise_sampler,
        );
        let scene_bind_group = pbr_pass.create_scene_bind_group(
            &device,
            &light_buffer,
            &shadow_view,
            &shadow_sampler,
            &ssao_blurred_view,
            &ssao_pbr_sampler,
            &cluster_manager.point_lights_buffer,
            &cluster_manager.metadata_buffer,
            &cluster_manager.light_indices_buffer,
            &cluster_manager.info_buffer,
            &csm_view,
            &csm_sampler,
            &local_shadow_atlas_view,
            &shadow_uniform_buffer,
            &vsm_pages_array_view,
            &vsm_sampler,
            &vsm_info_buffer,
            &ddgi_gpu.grid_buffer,
            &ddgi_gpu.probe_buffer,
            &sdf_gpu.scene_buffer,
            &sdf_gpu.view,
            &sdf_gpu.sampler,
            &vsm_shadow_history_view,
            &vsm_shadow_pbr_sampler,
        );

        let vg_system_inner = VGSystem::new(
            &device,
            VGConfig::default(),
            &camera_buffer,
            hdr_format,
            crate::pass::forward::DEPTH_FORMAT,
        );

        let depth_fmt = crate::pass::forward::DEPTH_FORMAT;
        let vg_vis_pass = VgVisPass::new(&device, &pbr_pass.frame_bgl, depth_fmt);
        let vg_resolve_pass = VgResolvePass::new(
            &device,
            hdr_format,
            &pbr_pass.frame_bgl,
            &pbr_pass.scene_bgl,
            msaa_sample_count,
        );
        let vg_shadow_pass = VgShadowPass::new(&device, SHADOW_FORMAT);
        let vg_vis_targets = create_vg_vis_targets(&device, internal_width, internal_height);
        let vg_material_buffer = build_material_storage(&[], &device, &queue);
        let vg_instance_buf = vg_system_inner.instance_buffer();
        let vg_meshlet_buf = vg_system_inner.meshlet_buffer();
        let vg_resolve_bind_group = vg_resolve_pass.create_resolve_bind_group(
            &device,
            &vg_vis_targets.id_view,
            &vg_vis_targets.normal_depth_view,
            &vg_vis_targets.uv_view,
            &depth_view,
            vg_instance_buf,
            vg_meshlet_buf,
            &vg_material_buffer,
        );
        let vg_shadow_instances_bg =
            vg_shadow_pass.create_instances_bind_group(&device, vg_instance_buf);

        // Pooled per-frame shadow resources: allocated once here, written via
        // queue.write_buffer each frame (removes per-frame buffer/BG churn).
        let cascade_matrix_buffers: Vec<wgpu::Buffer> = (0..4)
            .map(|i| {
                buffer::create_uniform_buffer(
                    &device,
                    &format!("cascade_matrix_buf{i}"),
                    &[0u8; 64],
                )
            })
            .collect();
        let cascade_matrix_bgs: Vec<wgpu::BindGroup> = cascade_matrix_buffers
            .iter()
            .enumerate()
            .map(|(i, buf)| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(&format!("cascade_matrix_bg{i}")),
                    layout: &shadow_pass.bind_group_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buf.as_entire_binding(),
                    }],
                })
            })
            .collect();
        let csm_layer_views: Vec<wgpu::TextureView> = (0..4)
            .map(|i| {
                csm_texture
                    .create_view(&wgpu::TextureViewDescriptor {
                        label: Some(&format!("csm_layer{i}_view")),
                        base_array_layer: i,
                        array_layer_count: Some(1),
                        dimension: Some(wgpu::TextureViewDimension::D2),
                        ..Default::default()
                    })
            })
            .collect();
        use crate::shadow::atlas_allocator::MAX_CASTERS;
        // One uniform + bind group per cube face per caster. Sharing 6 buffers
        // across casters races: Queue::write_buffer batches before the encoder,
        // so every face pass would sample the LAST caster's matrices.
        let face_slots = (MAX_CASTERS as usize) * 6;
        let face_matrix_buffers: Vec<wgpu::Buffer> = (0..face_slots)
            .map(|i| {
                buffer::create_uniform_buffer(
                    &device,
                    &format!("face_matrix_buf{i}"),
                    &[0u8; 64],
                )
            })
            .collect();
        let face_matrix_bgs: Vec<wgpu::BindGroup> = face_matrix_buffers
            .iter()
            .enumerate()
            .map(|(i, buf)| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(&format!("face_matrix_bg{i}")),
                    layout: &shadow_pass.bind_group_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buf.as_entire_binding(),
                    }],
                })
            })
            .collect();

        // ── VSM resources (directional; activated via `set_shadow_mode`) ──
        let vsm_depth_pass = crate::shadow::depth_pass::VsmDepthPass::new(&device);

        let info_size = std::mem::size_of::<crate::shadow::vsm_clipmap::VsmGpuInfo>() as u64;
        let vsm_info_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vsm_gpu_info"),
            size: info_size.max(16),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let vsm_pageinfo_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vsm_pageinfo_dyn"),
            // One 256-byte dynamic-offset slot per dirty page. Clipmap is
            // 8×16×16 = 2048 virtual pages; we only dirty those covering
            // scene AABBs, but size for a full invalidate just in case.
            size: 2048 * 256,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let vsm_pageinfo_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vsm_pageinfo_bg_dyn"),
            layout: &vsm_depth_pass.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: model_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &vsm_pageinfo_buffer,
                        offset: 0,
                        size: wgpu::BufferSize::new(256),
                    }),
                },
            ],
        });

        Self {
            device,
            queue,
            surface,
            config,
            render_scale,
            internal_width,
            internal_height,
            shared_layouts,
            pbr_frame_layouts,
            pbr_pass,
            shadow_pass,
            normal_pass,
            ssao_pass,
            camera_buffer,
            camera_uniform,
            camera_near: 0.1,
            camera_far: 200.0,
            light_uniform,
            light_buffer,
            shader_debug_buffer,
            frame_bind_group,
            scene_bind_group,
            msaa_sample_count,
            hdr_msaa_target_count,
            hdr_texture,
            hdr_view,
            hdr_msaa_texture,
            hdr_msaa_view,
            tone_mapping_pipeline,
            tone_mapping_bind_group,
            tonemap_uniform_buffer,
            exposure: 1.0,
            rcas_sharpness: 0.35,
            hdr_sampler,
            normal_texture,
            normal_view,
            normal_bind_group,
            ssao_raw_texture,
            ssao_raw_view,
            ssao_half_blurred_texture,
            ssao_half_blurred_view,
            ssao_blurred_texture,
            ssao_blurred_view,
            ssao_bind_group,
            ssao_blur_bind_group,
            ssao_upsample_bind_group,
            ssao_pbr_sampler,
            ssao_enabled: true,
            vsm_shadow_pass,
            vsm_shadow_raw_texture,
            vsm_shadow_raw_view,
            vsm_shadow_filtered_texture,
            vsm_shadow_filtered_view,
            vsm_shadow_history_texture,
            vsm_shadow_history_view,
            vsm_shadow_trace_bind_group,
            vsm_shadow_temporal_bind_group,
            vsm_shadow_pbr_sampler,
            depth_texture,
            depth_view,
            depth_msaa_texture,
            depth_msaa_view,
            prev_depth_texture,
            prev_depth_view,
            shadow_view,
            shadow_sampler,
            csm_texture,
            csm_view,
            csm_sampler,
            local_shadow_atlas,
            local_shadow_atlas_view,
            shadow_uniform,
            shadow_uniform_buffer,
            registry,
            fallback_texture_bg,
            fallback_normal_view: fallback_normal.view,
            fallback_mr_view: fallback_mr.view,
            common_sampler,
            material_blend_flags: Vec::new(),
            model_buffer,
            model_bind_group,
            shadow_model_bind_group,
            cluster_manager,
            lighting_mode: LightingMode::default(),
            scatter_light_pass,
            scatter_light_bind_group,
            aa_sharp,
            msaa_resolve_pass,
            msaa_resolve_bg,
            smaa_pass,
            ldr_texture,
            ldr_view,
            smaa_out_texture,
            smaa_out_view,
            tone_mapping_ldr_pipeline,
            tone_mapping_ldr_bg,
            rcas_present_pipeline,
            tone_mapping_rcas_bg,
            tonemap_rcas_uniform_buffer,
            bloom_pass,
            bloom_enabled: true,
            bloom_mip_textures,
            bloom_mip_views,
            bloom_scratch_texture,
            bloom_scratch_view,
            bloom_down_bgs,
            bloom_up_bgs,
            bloom_composite_bg,
            ddgi_gpu,
            sdf_gpu,
            blue_noise_texture,
            blue_noise_view,
            blue_noise_sampler,
            hiz,
            use_compressed_vertices: true,
            ibl_processor,
            ibl_resource,
            skybox_pass: None,
            transparent_pass,
            debug_lines_pass,
            current_debug_lines: Vec::new(),
            camera_bind_group,
            taa_enabled: true,
            taa_frame_count: 0,
            taa_prev_vp_unjittered: Mat4::IDENTITY,
            taa_pass,
            velocity_pass,
            taa_history_tex: [taa_history_tex0, taa_history_tex1],
            taa_history_view: [taa_history_view0, taa_history_view1],
            taa_blend_alpha,
            velocity_bg,
            taa_history_bgs,
            taa_tone_bgs,
            taa_velocity_tex,
            taa_velocity_view,
            taa_history_read_idx: 0,
            csm_state,
            // Virtual Geometry is initialised by default so the Nanite-like pipeline
            // is available without extra setup. It remains a no-op unless the scene
            // spawns entities with `VirtualMesh`.
            vg_system: Some(vg_system_inner),
            vg_vis_pass,
            vg_resolve_pass,
            vg_shadow_pass,
            vg_vis_targets,
            vg_material_buffer,
            vg_resolve_bind_group,
            vg_shadow_instances_bg,
            vg_use_vis_path: true,
            current_point_lights: Vec::new(),
            vsm_depth_pass,
            vsm_clipmap,
            vsm_pages_tex,
            vsm_pages_views,
            vsm_pageinfo_buffer,
            vsm_pageinfo_bg,
            vsm_info_buffer,
            vsm_sampler,
            vsm_enabled: true,
            vsm_screen_shadows: true,
            vsm_dirty_pages_last_frame: 0,
            smrt_config: crate::shadow::smrt::SmrtQuality::Quality.config(),
            vsm_shadow_frame_count: 0,
            vsm_shadow_temporal_alpha: 0.15,
            vsm_warm: false,
            prev_dynamic_centers: Vec::new(),
            vsm_pages_array_view,
            vg_bind_groups_dirty: true,
            cascade_matrix_buffers,
            cascade_matrix_bgs,
            csm_layer_views,
            face_matrix_buffers,
            face_matrix_bgs,
            particle_system: None,
            particle_sim_uniforms: ParticleSimUniforms::default(),
            reactive_texture: None,
            reactive_view: None,
            mt_command_recording: true,
            model_uniform_scratch: Vec::new(),
            main_vis_scratch: Vec::new(),
            vg_sync_scratch: Vec::new(),
            staging_ring,
        }
    }

    // ── MSAA helpers ────────────────────────────────────────────────────────

    /// Which HDR geometry pass performs the MSAA resolve (when `msaa_sample_count > 1`).
    fn hdr_geometry_resolve_pass(
        &self,
        objects: &[RenderObject],
        visibility: &[bool],
    ) -> HdrGeometryResolvePass {
        if self.aa_sharp && self.msaa_sample_count > 1 {
            return HdrGeometryResolvePass::SmartResolve;
        }
        if self.has_alpha_objects(objects, visibility) {
            HdrGeometryResolvePass::Transparent
        } else if self.vg_use_vis_path
            && self
                .vg_system
                .as_ref()
                .map(|v| v.has_content())
                .unwrap_or(false)
        {
            HdrGeometryResolvePass::VgResolve
        } else if self
            .vg_system
            .as_ref()
            .map(|v| v.has_content())
            .unwrap_or(false)
        {
            HdrGeometryResolvePass::VgDebug
        } else {
            HdrGeometryResolvePass::Forward
        }
    }

    fn geometry_depth_view(&self) -> &wgpu::TextureView {
        if self.msaa_sample_count > 1 {
            &self.depth_msaa_view
        } else {
            &self.depth_view
        }
    }

    fn geometry_depth_attachment<'a>(
        &'a self,
        load: wgpu::LoadOp<f32>,
    ) -> wgpu::RenderPassDepthStencilAttachment<'a> {
        wgpu::RenderPassDepthStencilAttachment {
            view: self.geometry_depth_view(),
            depth_ops: Some(wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }
    }

    fn hdr_color_attachment<'a>(
        hdr_view: &'a wgpu::TextureView,
        hdr_msaa_view: &'a wgpu::TextureView,
        msaa_sample_count: u32,
        load: wgpu::LoadOp<wgpu::Color>,
        resolve_pass: HdrGeometryResolvePass,
        this_pass: HdrGeometryResolvePass,
    ) -> wgpu::RenderPassColorAttachment<'a> {
        let resolve = msaa_sample_count > 1 && resolve_pass == this_pass;
        if msaa_sample_count > 1 {
            wgpu::RenderPassColorAttachment {
                view: hdr_msaa_view,
                resolve_target: if resolve { Some(hdr_view) } else { None },
                ops: wgpu::Operations {
                    load,
                    store: if resolve {
                        wgpu::StoreOp::Discard
                    } else {
                        wgpu::StoreOp::Store
                    },
                },
            }
        } else {
            wgpu::RenderPassColorAttachment {
                view: hdr_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            }
        }
    }

    fn sync_msaa_sample_count(&mut self) {
        let new_count = if self.aa_sharp || self.taa_enabled {
            self.hdr_msaa_target_count
        } else {
            1
        };
        if new_count == self.msaa_sample_count {
            return;
        }
        self.msaa_sample_count = new_count;
        self.recreate_geometry_pipelines();
        if self.msaa_sample_count > 1 {
            let (mt, mv) = create_hdr_msaa_texture(
                &self.device,
                self.internal_width,
                self.internal_height,
                self.msaa_sample_count,
            );
            self.hdr_msaa_texture = mt;
            self.hdr_msaa_view = mv;
            let (dt, dv) = create_depth_msaa_texture(
                &self.device,
                self.internal_width,
                self.internal_height,
                self.msaa_sample_count,
            );
            self.depth_msaa_texture = dt;
            self.depth_msaa_view = dv;
            self.msaa_resolve_pass.set_sample_count(&self.queue, self.msaa_sample_count);
            self.msaa_resolve_bg = self.msaa_resolve_pass.create_bind_group(
                &self.device, &self.hdr_msaa_view,
            );
        }
    }

    fn recreate_geometry_pipelines(&mut self) {
        let hdr_format = wgpu::TextureFormat::Rgba16Float;
        let sc = self.msaa_sample_count;
        self.pbr_pass = PbrPass::new(&self.device, hdr_format, sc);
        self.transparent_pass =
            TransparentPass::new(&self.device, &self.pbr_pass, hdr_format, sc);
        self.debug_lines_pass =
            DebugLinesPass::new(&self.device, &self.shared_layouts.camera_bgl, sc);
        self.vg_resolve_pass = VgResolvePass::new(
            &self.device,
            hdr_format,
            &self.pbr_pass.frame_bgl,
            &self.pbr_pass.scene_bgl,
            sc,
        );
        if let Some(sb) = &mut self.skybox_pass {
            *sb = SkyboxPass::new(
                &self.device,
                &self.camera_buffer,
                &self.ibl_resource.environment_view,
                &self.ibl_processor.sampler,
                hdr_format,
                sc,
            );
        }
        self.update_global_bind_group();
        if let Some(vg) = &self.vg_system {
            self.vg_resolve_bind_group = self.vg_resolve_pass.create_resolve_bind_group(
                &self.device,
                &self.vg_vis_targets.id_view,
                &self.vg_vis_targets.normal_depth_view,
                &self.vg_vis_targets.uv_view,
                &self.depth_view,
                vg.instance_buffer(),
                vg.meshlet_buffer(),
                &self.vg_material_buffer,
            );
        }
    }

    // ── TAA helpers ─────────────────────────────────────────────────────────

    /// Enables or disables Temporal Anti-Aliasing.
    ///
    /// When enabled the pipeline inserts a velocity pass and a TAA accumulation
    /// pass between the main forward pass and tone mapping.  The jitter is
    /// injected automatically inside [`render_frame`]; callers keep their
    /// `update_camera_buffer` workflow unchanged.
    ///
    /// Disabling TAA resets the frame counter so history is correctly
    /// re-initialised if TAA is re-enabled later.
    pub fn enable_taa(&mut self, enabled: bool) {
        if enabled {
            self.aa_sharp = false;
            self.taa_frame_count = 0;
            self.taa_history_read_idx = 0;
            self.taa_prev_vp_unjittered = Mat4::from_cols_array_2d(&self.camera_uniform.view_proj);
        } else if self.taa_enabled {
            self.taa_frame_count = 0;
            self.taa_history_read_idx = 0;
        }
        self.taa_enabled = enabled;
        self.sync_msaa_sample_count();
    }

    /// Enables the Sharp AA pipeline (MSAA + Smart Resolve + SMAA + RCAS).
    /// Mutually exclusive with TAA.
    pub fn set_aa_sharp(&mut self, enabled: bool) {
        self.aa_sharp = enabled;
        if enabled {
            self.taa_enabled = false;
            self.taa_frame_count = 0;
            self.taa_history_read_idx = 0;
        }
        self.sync_msaa_sample_count();
    }

    /// Sets TAA blend factor (weight of current frame).
    ///
    /// - Lower values reduce aliasing/shimmer, but increase ghosting risk.
    /// - Typical range: 0.04–0.10.
    pub fn set_taa_blend_alpha(&mut self, alpha: f32) {
        self.taa_blend_alpha = alpha.clamp(0.02, 0.20);
        // Clear accumulation once so the new blend behaviour settles quickly.
        self.taa_frame_count = 0;
        self.taa_history_read_idx = 0;
    }

    /// Sets RCAS sharpening strength after TAA (0 = max sharp, ~0.5 = mild).
    pub fn set_rcas_sharpness(&mut self, sharpness_stops: f32) {
        self.rcas_sharpness = sharpness_stops.clamp(0.0, 1.0);
    }

    /// Reactive mask view for TAA (real R8 texture or 1×1 fallback).
    pub fn reactive_mask_view(&self) -> &wgpu::TextureView {
        self.reactive_view
            .as_ref()
            .unwrap_or(&self.taa_pass.reactive_fallback_view)
    }

    /// Allocates the internal-resolution R8 reactive mask used by particles / TAA.
    pub fn ensure_reactive_texture(&mut self) {
        let size = wgpu::Extent3d {
            width: self.internal_width.max(1),
            height: self.internal_height.max(1),
            depth_or_array_layers: 1,
        };
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("reactive_mask"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        self.reactive_texture = Some(tex);
        self.reactive_view = Some(view);
        self.rebuild_taa_history_bind_groups();
    }

    fn rebuild_taa_history_bind_groups(&mut self) {
        let reactive = self.reactive_mask_view();
        self.taa_history_bgs = [
            self.taa_pass.create_bind_group(
                &self.device,
                &self.hdr_view,
                &self.taa_velocity_view,
                &self.taa_history_view[0],
                reactive,
                &self.prev_depth_view,
            ),
            self.taa_pass.create_bind_group(
                &self.device,
                &self.hdr_view,
                &self.taa_velocity_view,
                &self.taa_history_view[1],
                reactive,
                &self.prev_depth_view,
            ),
        ];
    }

    /// Creates the optional GPU particle system (`capacity` living particles).
    pub fn init_particle_system(&mut self, capacity: u32) {
        let hdr_format = wgpu::TextureFormat::Rgba16Float;
        self.particle_system =
            Some(ParticleGpuSystem::new(&self.device, capacity, hdr_format));
        self.ensure_reactive_texture();
    }

    /// Upload CPU-simulated particles from `redox_vfx` for GPU billboard draw.
    pub fn upload_particles_from_vfx(&mut self, system: &ParticleSystem) {
        self.particle_sim_uniforms = system.sim_uniforms(1.0 / 60.0);
        if let Some(ps) = &mut self.particle_system {
            ps.upload_from_vfx(
                &self.device,
                &self.queue,
                system,
                &self.camera_uniform,
            );
        }
    }

    /// Enable or disable multi-threaded command recording (CSM + prepass).
    pub fn set_mt_command_recording(&mut self, enabled: bool) {
        self.mt_command_recording = enabled;
    }

    /// Sets debug lines to draw this frame (e.g. from [`redox_audio::AudioDebugDraw`]).
    pub fn set_debug_lines(&mut self, lines: Vec<(Vec3, Vec3, bool)>) {
        self.current_debug_lines = lines;
    }

    /// Uploads current debug lines to the GPU. Call before starting the render pass that will draw them.
    pub fn upload_debug_lines(&mut self) {
        let lines = std::mem::take(&mut self.current_debug_lines);
        if !lines.is_empty() {
            self.debug_lines_pass.upload_lines(&self.queue, &lines);
        }
    }

    /// Records drawing the debug lines uploaded by [`upload_debug_lines`](Self::upload_debug_lines).
    /// Call after [`record_draw`](Self::record_draw) in the same render pass. Uses only `&self`.
    pub fn draw_debug_lines<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        self.debug_lines_pass.draw(&self.camera_bind_group, pass);
    }

    /// If debug lines were set this frame, uploads and records drawing them into `pass`.
    /// Call after [`record_draw`](Self::record_draw) in the same render pass. Clears the line list.
    pub fn draw_debug_lines_if_any<'a>(&'a mut self, pass: &mut wgpu::RenderPass<'a>) {
        let lines = std::mem::take(&mut self.current_debug_lines);
        if lines.is_empty() {
            return;
        }
        self.debug_lines_pass.upload_lines(&self.queue, &lines);
        self.debug_lines_pass.draw(&self.camera_bind_group, pass);
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    pub fn surface(&self) -> &wgpu::Surface<'_> {
        self.surface
            .as_ref()
            .expect("offscreen Renderer has no window surface")
    }

    pub fn config(&self) -> &wgpu::SurfaceConfiguration {
        &self.config
    }

    /// Sets the internal render scale used by TAAU.
    ///
    /// - < 1.0: TAAU upscaling (faster, blurrier, more aliasing risk)
    /// - = 1.0: regular TAA (no upscale)
    /// - > 1.0: temporal supersampling (sharper, fewer stair-steps, slower)
    ///
    /// This recreates internal-resolution textures (depth/HDR/normal/SSAO/velocity)
    /// and forces a TAA history reset on the next frame.
    pub fn set_render_scale(&mut self, scale: f32) {
        self.render_scale = scale.clamp(0.5, 2.0);
        // Recreate size-dependent resources using the current output resolution.
        let w = self.config.width;
        let h = self.config.height;
        self.resize(w, h);
        // If TAA is enabled, ensure history is clean after scale change.
        self.taa_frame_count = 0;
        self.taa_history_read_idx = 0;
        self.reset_vsm_shadow_temporal();
    }

    /// Selects the forward lighting path (clustered vs ScatterLight).
    pub fn set_lighting_mode(&mut self, mode: LightingMode) {
        self.lighting_mode = mode;
    }

    /// Returns `true` when ScatterLight stochastic sampling is active.
    pub fn uses_scatter_light(&self) -> bool {
        self.lighting_mode == LightingMode::ScatterLight
    }

    /// Returns `true` when TAA upscales from sub-native resolution (TSR path).
    pub fn uses_tsr(&self) -> bool {
        self.taa_enabled && self.render_scale < 0.999
    }

    /// Reconfigure the surface and depth buffer after a window resize.
    pub fn resize(&mut self, new_width: u32, new_height: u32) {
        if new_width == 0 || new_height == 0 {
            return;
        }
        self.config.width = new_width;
        self.config.height = new_height;
        if let Some(ref surface) = self.surface {
            surface.configure(&self.device, &self.config);
        }

        // Recompute internal resolution (TAAU input size)
        self.internal_width = ((new_width as f32) * self.render_scale).round().max(1.0) as u32;
        self.internal_height = ((new_height as f32) * self.render_scale).round().max(1.0) as u32;

        if self.reactive_texture.is_some() {
            self.ensure_reactive_texture();
        }

        // Depth (internal)
        let (dt, dv) = create_depth_texture(&self.device, self.internal_width, self.internal_height);
        self.depth_texture = dt;
        self.depth_view = dv;
        let (pdt, pdv) = create_prev_depth_texture(
            &self.device,
            &self.queue,
            self.internal_width,
            self.internal_height,
        );
        self.prev_depth_texture = pdt;
        self.prev_depth_view = pdv;

        // HDR (internal)
        let (ht, hv, hs) = create_hdr_texture(&self.device, self.internal_width, self.internal_height);
        self.hdr_texture = ht;
        self.hdr_view = hv;
        self.hdr_sampler = hs;

        if self.msaa_sample_count > 1 {
            let (mt, mv) = create_hdr_msaa_texture(
                &self.device,
                self.internal_width,
                self.internal_height,
                self.msaa_sample_count,
            );
            self.hdr_msaa_texture = mt;
            self.hdr_msaa_view = mv;
            let (dmt, dmv) = create_depth_msaa_texture(
                &self.device,
                self.internal_width,
                self.internal_height,
                self.msaa_sample_count,
            );
            self.depth_msaa_texture = dmt;
            self.depth_msaa_view = dmv;
            self.msaa_resolve_bg = self.msaa_resolve_pass.create_bind_group(
                &self.device, &self.hdr_msaa_view,
            );
        }

        // Sharp AA intermediates (output resolution)
        let (lt, lv) = create_sharp_intermediate(
            &self.device, new_width, new_height, "sharp_ldr",
        );
        self.ldr_texture = lt;
        self.ldr_view = lv;
        let (st, sv) = create_sharp_intermediate(
            &self.device, new_width, new_height, "sharp_smaa_out",
        );
        self.smaa_out_texture = st;
        self.smaa_out_view = sv;
        self.smaa_pass.resize(&self.device, new_width, new_height);

        // Tone Mapping Bind Group (samples from the new HDR texture).
        self.tone_mapping_bind_group = create_tonemap_bind_group(
            &self.device,
            &self.tone_mapping_pipeline,
            &self.hdr_view,
            &self.bloom_mip_views[0],
            &self.hdr_sampler,
            &self.tonemap_uniform_buffer,
        );

        // Normal + GTAO textures (half-res compute, full-res upsample)
        let half_width = (self.internal_width / 2).max(1);
        let half_height = (self.internal_height / 2).max(1);
        let gtao_fmt = wgpu::TextureFormat::Rgba16Float;

        let (nt, nv) = create_normal_ssao_texture(
            &self.device,
            self.internal_width,
            self.internal_height,
            wgpu::TextureFormat::Rgba16Float,
            "normal",
        );
        self.normal_texture = nt;
        self.normal_view = nv;

        let (sr, sv) = create_normal_ssao_texture(
            &self.device,
            half_width,
            half_height,
            gtao_fmt,
            "gtao_raw",
        );
        self.ssao_raw_texture = sr;
        self.ssao_raw_view = sv;

        let (sh, shv) = create_normal_ssao_texture(
            &self.device,
            half_width,
            half_height,
            gtao_fmt,
            "gtao_half_blurred",
        );
        self.ssao_half_blurred_texture = sh;
        self.ssao_half_blurred_view = shv;

        let (sb, sbv) = create_normal_ssao_texture(
            &self.device,
            self.internal_width,
            self.internal_height,
            gtao_fmt,
            "gtao_blurred",
        );
        self.ssao_blurred_texture = sb;
        self.ssao_blurred_view = sbv;

        let (vsr, vsv) = create_normal_ssao_texture(
            &self.device,
            self.internal_width,
            self.internal_height,
            wgpu::TextureFormat::R16Float,
            "vsm_shadow_raw",
        );
        self.vsm_shadow_raw_texture = vsr;
        self.vsm_shadow_raw_view = vsv;
        let (vsf, vsfv) = create_normal_ssao_texture(
            &self.device,
            self.internal_width,
            self.internal_height,
            wgpu::TextureFormat::R16Float,
            "vsm_shadow_filtered",
        );
        self.vsm_shadow_filtered_texture = vsf;
        self.vsm_shadow_filtered_view = vsfv;
        let (vsh, vshv) = create_normal_ssao_texture(
            &self.device,
            self.internal_width,
            self.internal_height,
            wgpu::TextureFormat::R16Float,
            "vsm_shadow_history",
        );
        self.vsm_shadow_history_texture = vsh;
        self.vsm_shadow_history_view = vshv;

        self.normal_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("normal_bind_group"),
            layout: &self.normal_pass.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.model_buffer.as_entire_binding(),
                },
            ],
        });

        let ssao_sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ssao_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        self.ssao_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gtao_bind_group"),
            layout: &self.ssao_pass.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.normal_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &self.ssao_pass.noise_texture.view,
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&ssao_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.camera_buffer.as_entire_binding(),
                },
            ],
        });

        self.ssao_blur_bind_group = self.ssao_pass.create_blur_bind_group(
            &self.device,
            &self.ssao_raw_view,
            &self.common_sampler,
            &self.normal_view,
        );
        self.ssao_upsample_bind_group = self.ssao_pass.create_upsample_bind_group(
            &self.device,
            &self.ssao_half_blurred_view,
            &self.common_sampler,
            &self.normal_view,
        );

        self.vsm_shadow_trace_bind_group = self.vsm_shadow_pass.create_trace_bind_group(
            &self.device,
            &self.normal_view,
            &self.camera_buffer,
            &self.light_buffer,
            &self.vsm_pages_array_view,
            &self.vsm_sampler,
            &self.vsm_info_buffer,
            &self.blue_noise_view,
            &self.blue_noise_sampler,
        );
        self.vsm_shadow_temporal_bind_group = self.vsm_shadow_pass.create_temporal_bind_group(
            &self.device,
            &self.vsm_shadow_raw_view,
            &self.vsm_shadow_history_view,
            &self.normal_view,
            &self.camera_buffer,
            &self.common_sampler,
        );

        // Rebuild cluster buffers for the new internal dimensions so that the
        // lighting grid matches the shading resolution.
        self.hiz
            .resize(&self.device, self.internal_width, self.internal_height);

        self.cluster_manager.resize_for_screen(
            self.internal_width,
            self.internal_height,
            &self.device,
            &self.queue,
        );

        self.update_global_bind_group();

        // Recreate TAA history at output resolution (full-res)
        let (vh0, vv0) = create_history_texture(&self.device, new_width, new_height, "taa_history_0");
        let (vh1, vv1) = create_history_texture(&self.device, new_width, new_height, "taa_history_1");
        self.taa_history_tex = [vh0, vh1];
        self.taa_history_view = [vv0, vv1];
        // Velocity is generated at internal resolution.
        let (vt, vv) = create_velocity_texture(&self.device, self.internal_width, self.internal_height);
        self.taa_velocity_tex = vt;
        self.taa_velocity_view = vv;
        // Force history reset so stale data is not blended on the next frame
        self.taa_frame_count = 0;
        self.taa_history_read_idx = 0;
        self.reset_vsm_shadow_temporal();
        // Rebuild cached per-frame bind groups against the new textures.
        self.velocity_bg = self
            .velocity_pass
            .create_bind_group(&self.device, &self.depth_view);
        // Bind real reactive mask (not 1×1 fallback) after HDR/velocity recreate.
        self.rebuild_taa_history_bind_groups();
        let make_tone = |view: &wgpu::TextureView| {
            create_tonemap_bind_group(
                &self.device,
                &self.tone_mapping_pipeline,
                view,
                &self.bloom_mip_views[0],
                &self.hdr_sampler,
                &self.tonemap_uniform_buffer,
            )
        };
        self.taa_tone_bgs = [
            make_tone(&self.taa_history_view[0]),
            make_tone(&self.taa_history_view[1]),
        ];

        self.vg_vis_targets =
            create_vg_vis_targets(&self.device, self.internal_width, self.internal_height);
        self.rebuild_vg_bind_groups();

        let (bloom_mip_textures, bloom_mip_views) = create_bloom_mip_chain(
            &self.device,
            self.internal_width,
            self.internal_height,
            wgpu::TextureFormat::Rgba16Float,
        );
        let (bst, bsv) = create_bloom_texture(
            &self.device,
            self.internal_width,
            self.internal_height,
            wgpu::TextureFormat::Rgba16Float,
            "bloom_scratch",
        );
        self.bloom_mip_textures = bloom_mip_textures;
        self.bloom_mip_views = bloom_mip_views;
        self.bloom_scratch_texture = bst;
        self.bloom_scratch_view = bsv;
        self.rebuild_bloom_bind_groups();
        self.rebuild_sharp_bind_groups();

        log::info!("Resized to {}×{}", new_width, new_height);
    }

    fn rebuild_bloom_bind_groups(&mut self) {
        self.bloom_down_bgs = create_bloom_downsample_bgs(
            &self.bloom_pass,
            &self.device,
            &self.hdr_view,
            &self.bloom_mip_views,
            &self.hdr_sampler,
        );
        self.bloom_up_bgs = create_bloom_upsample_bgs(
            &self.bloom_pass,
            &self.device,
            &self.bloom_mip_views,
            &self.hdr_sampler,
        );
        self.bloom_composite_bg = self.bloom_pass.create_composite_bind_group(
            &self.device,
            &self.hdr_view,
            &self.bloom_mip_views[0],
            &self.hdr_sampler,
        );
        self.scatter_light_bind_group = self.scatter_light_pass.create_bind_group(
            &self.device,
            &self.hdr_view,
            &self.depth_view,
            &self.hdr_sampler,
            &self.cluster_manager.point_lights_buffer,
        );
    }

    fn rebuild_sharp_bind_groups(&mut self) {
        self.tone_mapping_ldr_bg = create_tonemap_bind_group(
            &self.device,
            &self.tone_mapping_ldr_pipeline,
            &self.hdr_view,
            &self.bloom_mip_views[0],
            &self.hdr_sampler,
            &self.tonemap_uniform_buffer,
        );
        self.tone_mapping_rcas_bg = create_tonemap_bind_group(
            &self.device,
            &self.rcas_present_pipeline,
            &self.smaa_out_view,
            &self.bloom_mip_views[0],
            &self.hdr_sampler,
            &self.tonemap_rcas_uniform_buffer,
        );
    }

    /// ScatterLight fullscreen pass on the HDR buffer (no bloom — bloom runs after TAA).
    fn record_scatter_light_pass(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if self.uses_scatter_light() {
            let uniform = ScatterLightUniform {
                num_lights: self.current_point_lights.len() as u32,
                sample_count: crate::pass::scatter_light::SCATTER_LIGHT_SAMPLE_COUNT,
                frame_index: self.taa_frame_count as u32,
                _pad: 0,
                resolution: [self.internal_width as f32, self.internal_height as f32],
            };
            self.queue.write_buffer(
                &self.scatter_light_pass.uniform_buffer,
                0,
                bytemuck::bytes_of(&uniform),
            );
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scatter_light_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.hdr_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            self.scatter_light_pass.draw(&mut pass, &self.scatter_light_bind_group);
        }
    }

    /// Dual-filter bloom mip chain from a clean HDR source (TAA output or raw HDR).
    fn record_bloom_chain(&self, encoder: &mut wgpu::CommandEncoder, source_view: &wgpu::TextureView) {
        if !self.bloom_enabled {
            return;
        }

        use crate::post::bloom::BLOOM_MIP_COUNT;

        let bloom_src_bg = self.bloom_pass.create_bind_group(
            &self.device,
            source_view,
            &self.hdr_sampler,
        );

        for i in 0..BLOOM_MIP_COUNT {
            let pipeline = if i == 0 {
                &self.bloom_pass.downsample_threshold_pipeline
            } else {
                &self.bloom_pass.downsample_pipeline
            };
            let bg = if i == 0 {
                &bloom_src_bg
            } else {
                &self.bloom_down_bgs[i]
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bloom_downsample"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.bloom_mip_views[i],
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bg, &[]);
            pass.draw(0..3, 0..1);
        }

        for i in (0..BLOOM_MIP_COUNT - 1).rev() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bloom_upsample"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.bloom_mip_views[i],
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.bloom_pass.upsample_pipeline);
            pass.set_bind_group(0, &self.bloom_up_bgs[i], &[]);
            pass.draw(0..3, 0..1);
        }
    }

    fn rebuild_vg_bind_groups(&mut self) {
        let materials: Vec<Material> = self.registry.legacy_materials().to_vec();
        self.vg_material_buffer =
            build_material_storage(&materials, &self.device, &self.queue);

        if let Some(vg) = &self.vg_system {
            self.vg_resolve_bind_group = self.vg_resolve_pass.create_resolve_bind_group(
                &self.device,
                &self.vg_vis_targets.id_view,
                &self.vg_vis_targets.normal_depth_view,
                &self.vg_vis_targets.uv_view,
                &self.depth_view,
                vg.instance_buffer(),
                vg.meshlet_buffer(),
                &self.vg_material_buffer,
            );
            self.vg_shadow_instances_bg = self
                .vg_shadow_pass
                .create_instances_bind_group(&self.device, vg.instance_buffer());
        }
        self.vg_bind_groups_dirty = false;
    }

    // ----- Resource upload helpers -----

    /// Uploads a CPU [`Mesh`] to the GPU and returns its slot index.
    pub fn upload_mesh(&mut self, mesh: &Mesh) -> usize {
        let vertex_bytes: Vec<u8> = if self.use_compressed_vertices {
            let compressed = compress_vertices(&mesh.vertices, &mesh.aabb);
            bytemuck::cast_slice(&compressed).to_vec()
        } else {
            bytemuck::cast_slice(&mesh.vertices).to_vec()
        };
        let vb = buffer::create_vertex_buffer(&self.device, "mesh_vb", &vertex_bytes);
        let ib = buffer::create_index_buffer(
            &self.device,
            "mesh_ib",
            bytemuck::cast_slice(&mesh.indices),
        );
        self.registry.insert_mesh_slot(GpuMesh {
            vertex_buffer: vb,
            index_buffer: ib,
            index_count: mesh.index_count(),
            local_bounds: mesh.aabb,
        })
    }

    /// Stores a material, creates its GPU resources, and returns its index.
    pub fn add_material(&mut self, material: Material) -> usize {
        let uniform = material.to_uniform(&self.device);
        let buffer = buffer::create_uniform_buffer(
            &self.device,
            "material_uniform",
            bytemuck::bytes_of(&uniform),
        );

        let albedo_view = material
            .texture_index
            .and_then(|idx| self.registry.legacy_texture(idx))
            .map(|t| &t.view)
            .unwrap_or_else(|| self.registry.legacy_texture(0).map(|t| &t.view).expect("fallback texture"));

        let normal_view = material
            .normal_texture_index
            .and_then(|idx| self.registry.legacy_texture(idx))
            .map(|t| &t.view)
            .unwrap_or(&self.fallback_normal_view);

        let mr_view = material
            .mr_texture_index
            .and_then(|idx| self.registry.legacy_texture(idx))
            .map(|t| &t.view)
            .unwrap_or(&self.fallback_mr_view);

        let bind_group = self.pbr_pass.create_material_bind_group(
            &self.device,
            &buffer,
            albedo_view,
            normal_view,
            mr_view,
            &self.common_sampler,
        );

        let idx = self.registry.push_legacy_material(material, buffer, bind_group);
        self.material_blend_flags.push(BlendMode::Opaque);
        self.vg_bind_groups_dirty = true;
        idx
    }

    /// Uploads a texture from raw bytes and returns its index.
    pub fn upload_texture(
        &mut self,
        bytes: &[u8],
        label: &str,
    ) -> Result<usize, image::ImageError> {
        let tex = Texture::from_bytes(
            &self.device,
            &self.queue,
            bytes,
            label,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        )?;
        let bg = self
            .shared_layouts
            .create_texture_bind_group(&self.device, &tex);
        Ok(self.registry.push_legacy_texture(tex, bg))
    }

    /// Uploads a texture with specific format (e.g. for normals/MR).
    pub fn upload_texture_with_format(
        &mut self,
        bytes: &[u8],
        label: &str,
        format: wgpu::TextureFormat,
    ) -> Result<usize, image::ImageError> {
        let tex = Texture::from_bytes(&self.device, &self.queue, bytes, label, format)?;
        let bg = self
            .shared_layouts
            .create_texture_bind_group(&self.device, &tex);
        Ok(self.registry.push_legacy_texture(tex, bg))
    }

    // ----- Asset-handle–based upload (for redox_asset integration) -----

    pub fn get_mesh_index(&self, handle: Handle<MeshData>) -> Option<usize> {
        self.registry.mesh_index_for(handle.id())
    }

    pub fn get_mesh_bounds(&self, slot: usize) -> Option<redox_math::Aabb> {
        self.registry.mesh_by_slot(slot).map(|m| m.local_bounds)
    }

    pub fn get_texture_index(&self, handle: Handle<TextureData>) -> Option<usize> {
        self.registry.texture_index_for(handle.id())
    }

    pub fn get_material_index(&self, handle: Handle<MaterialData>) -> Option<usize> {
        self.registry.material_index_for(handle.id())
    }

    pub fn add_mesh_from_asset(&mut self, handle: Handle<MeshData>, mesh: &Mesh) -> Option<usize> {
        if self.registry.has_mesh(handle.id()) {
            return self.registry.mesh_index_for(handle.id());
        }
        let gpu_mesh = {
            let vb = buffer::create_vertex_buffer(
                &self.device,
                "mesh_vb",
                bytemuck::cast_slice(&mesh.vertices),
            );
            let ib = buffer::create_index_buffer(
                &self.device,
                "mesh_ib",
                bytemuck::cast_slice(&mesh.indices),
            );
            GpuMesh {
                vertex_buffer: vb,
                index_buffer: ib,
                index_count: mesh.index_count(),
                local_bounds: mesh.aabb,
            }
        };
        Some(self.registry.insert_mesh(handle.id(), gpu_mesh))
    }

    pub fn add_texture_from_asset(
        &mut self,
        handle: Handle<TextureData>,
        img: &image::DynamicImage,
        label: &str,
    ) -> Result<Option<usize>, image::ImageError> {
        if self.registry.has_texture(handle.id()) {
            return Ok(self.registry.texture_index_for(handle.id()));
        }
        let tex = Texture::from_image(
            &self.device,
            &self.queue,
            img,
            label,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        )?;
        let bg = self
            .shared_layouts
            .create_texture_bind_group(&self.device, &tex);
        let idx = self.registry.insert_texture(handle.id(), crate::registry::TextureSlot {
            texture: tex,
            bind_group: Some(bg),
        });
        Ok(Some(idx))
    }

    pub fn add_material_from_asset(
        &mut self,
        handle: Handle<MaterialData>,
        data: &MaterialData,
    ) -> Option<usize> {
        if self.registry.has_material(handle.id()) {
            return self.registry.material_index_for(handle.id());
        }
        let texture_index = data
            .albedo_handle
            .as_ref()
            .and_then(|h| self.registry.texture_index_for(h.id()));
        let normal_texture_index = data
            .normal_handle
            .as_ref()
            .and_then(|h| self.registry.texture_index_for(h.id()));
        let mr_texture_index = data
            .mr_handle
            .as_ref()
            .and_then(|h| self.registry.texture_index_for(h.id()));
        let material = Material {
            base_color: data.base_color,
            texture_index,
            normal_texture_index,
            mr_texture_index,
            metallic: data.metallic,
            roughness: data.roughness,
            emissive: data.emissive,
            layers: data.layers,
            sheen: data.sheen,
        };
        let idx = self.add_material(material);
        self.material_blend_flags[idx] = data.blend;
        self.registry.link_material_index(handle.id(), idx);
        Some(idx)
    }

    // ----- Rendering -----

    /// Renders a single frame given a list of [`RenderObject`]s.
    ///
    pub fn begin_pass<'a>(
        &'a self,
        encoder: &'a mut wgpu::CommandEncoder,
    ) -> wgpu::RenderPass<'a> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("forward_pass"),
            color_attachments: &[Some(Self::hdr_color_attachment(
                &self.hdr_view,
                &self.hdr_msaa_view,
                self.msaa_sample_count,
                wgpu::LoadOp::Clear(wgpu::Color {
                    r: 0.22,
                    g: 0.26,
                    b: 0.34,
                    a: 1.0,
                }),
                HdrGeometryResolvePass::Forward,
                HdrGeometryResolvePass::Forward,
            ))],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        })
    }

    pub fn record_draw<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        objects: &[RenderObject],
        visibility: &[bool],
    ) {
        debug_assert!(visibility.is_empty() || visibility.len() == objects.len());
        render_pass.set_pipeline(&self.pbr_pass.pipeline);
        render_pass.set_bind_group(0, &self.frame_bind_group, &[]);
        render_pass.set_bind_group(1, &self.scene_bind_group, &[]);
        render_pass.set_bind_group(3, &self.model_bind_group, &[]);

        for (i, obj) in objects.iter().enumerate() {
            if !visibility.is_empty() && !visibility[i] {
                continue;
            }
            // Material bind group (Group 2)
            let material_bg = self
                .registry
                .legacy_material_bind_group(obj.material_index)
                .unwrap_or_else(|| self.registry.legacy_material_bind_group(0).expect("fallback material"));

            render_pass.set_bind_group(2, material_bg, &[]);
            if let Some(gpu_mesh) = self.registry.mesh_by_slot(obj.mesh_index) {
                render_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                render_pass
                    .set_index_buffer(gpu_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.draw_indexed(0..gpu_mesh.index_count, 0, (i as u32)..(i as u32 + 1));
            }
        }
    }

    /// Returns true when any visible object uses an alpha-blended material.
    ///
    /// Cheap pre-check so the transparent render pass is not opened for
    /// fully opaque frames.
    pub fn has_alpha_objects(&self, objects: &[RenderObject], visibility: &[bool]) -> bool {
        objects.iter().enumerate().any(|(i, obj)| {
            (visibility.is_empty() || visibility[i])
                && self
                    .material_blend_flags
                    .get(obj.material_index)
                    .copied()
                    .unwrap_or(BlendMode::Opaque)
                    == BlendMode::Alpha
        })
    }

    /// Draws alpha-blended objects into the HDR target with depth read-only.
    ///
    /// Objects are sorted back-to-front by distance from the camera
    /// (farthest drawn first) so alpha compositing order is correct.
    /// Instance indices stay GLOBAL — they index the model storage buffer,
    /// which holds all objects, not just the transparent ones.
    pub fn record_draw_transparent<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        objects: &[RenderObject],
        visibility: &[bool],
    ) {
        let cam_pos = Vec3::new(
            self.camera_uniform.camera_pos[0],
            self.camera_uniform.camera_pos[1],
            self.camera_uniform.camera_pos[2],
        );

        // Collect visible alpha-blended object indices with their distance.
        let mut order: Vec<(usize, f32)> = objects
            .iter()
            .enumerate()
            .filter(|(i, obj)| {
                (visibility.is_empty() || visibility[*i])
                    && self
                        .material_blend_flags
                        .get(obj.material_index)
                        .copied()
                        .unwrap_or(BlendMode::Opaque)
                        == BlendMode::Alpha
            })
            .map(|(i, obj)| {
                let c = obj.bounds.center();
                (i, (c - cam_pos).length_squared())
            })
            .collect();

        // Farthest first (descending distance).
        order.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        if order.is_empty() {
            return;
        }

        render_pass.set_pipeline(&self.transparent_pass.pipeline);
        render_pass.set_bind_group(0, &self.frame_bind_group, &[]);
        render_pass.set_bind_group(1, &self.scene_bind_group, &[]);
        render_pass.set_bind_group(3, &self.model_bind_group, &[]);

        for (i, _) in order {
            let obj = &objects[i];
            let material_bg = self
                .registry
                .legacy_material_bind_group(obj.material_index)
                .unwrap_or_else(|| self.registry.legacy_material_bind_group(0).expect("fallback material"));

            render_pass.set_bind_group(2, material_bg, &[]);
            if let Some(gpu_mesh) = self.registry.mesh_by_slot(obj.mesh_index) {
                render_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                render_pass
                    .set_index_buffer(gpu_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.draw_indexed(0..gpu_mesh.index_count, 0, (i as u32)..(i as u32 + 1));
            }
        }
    }

    /// Sets the environmental skybox and generates IBL maps.
    pub fn set_environment(&mut self, equirect: &Texture) {
        self.ibl_resource =
            IBLResource::from_equirect(&self.device, &self.queue, &self.ibl_processor, equirect);
        self.update_shader_debug_buffer(ShaderDebugVizMode::None);
        self.update_global_bind_group();
        // The skybox samples the environment view directly: rebind it so a
        // live skybox picks up the freshly generated cubemap.
        if let Some(sb) = &mut self.skybox_pass {
            sb.refresh_bind_group(
                &self.device,
                &self.camera_buffer,
                &self.ibl_resource.environment_view,
                &self.ibl_processor.sampler,
            );
        }
    }

    /// Sets the linear exposure multiplier used by the ACES tone mapper.
    ///
    /// `1.0` is neutral; larger values brighten the frame. Takes effect on
    /// the next rendered frame (the uniform is written per frame).
    pub fn set_vsm_enabled(&mut self, enabled: bool) {
        if enabled && !self.vsm_enabled {
            // Drop stale page cache so the next frame rebuilds with current matrices.
            self.vsm_clipmap
                .invalidate(&[crate::shadow::cache::InvalidationEvent::Full]);
            self.vsm_warm = false;
            self.reset_vsm_shadow_temporal();
        }
        self.vsm_enabled = enabled;
    }
    /// Clears VSM screen-space shadow temporal history (first frame = no blend).
    fn reset_vsm_shadow_temporal(&mut self) {
        self.vsm_shadow_frame_count = 0;
    }

    /// Enables screen-space VSM trace (1 ray + temporal) vs inline multi-ray PBR.
    pub fn set_vsm_screen_shadows(&mut self, enabled: bool) {
        if enabled != self.vsm_screen_shadows {
            self.vsm_screen_shadows = enabled;
            self.reset_vsm_shadow_temporal();
            // Keep LightUniform flag in sync next frame (written in render_frame)
            self.light_uniform.vsm_screen_shadows = u32::from(enabled);
        }
    }


    /// Sets SMRT quality for VSM soft shadows (rays × samples per pixel).
    pub fn set_smrt_quality(&mut self, quality: crate::shadow::smrt::SmrtQuality) {
        self.smrt_config = quality.config();
    }

    /// Sets temporal blend alpha for VSM screen shadows (weight of current frame).
    /// Lower = smoother but more ghosting; typical 0.05..0.20. Default 0.08 (92% history).
    pub fn set_vsm_temporal_alpha(&mut self, alpha: f32) {
        self.vsm_shadow_temporal_alpha = alpha.clamp(0.05, 0.20);
    }


    pub fn set_ssao_enabled(&mut self, enabled: bool) {
        self.ssao_enabled = enabled;
    }

    pub fn set_exposure(&mut self, exposure: f32) {
        self.exposure = exposure.max(0.0);
    }

    /// Enables or disables the skybox background pass.
    ///
    /// When enabled, the pass samples `ibl_resource.environment_view` in the
    /// forward pass before opaque geometry. While only the dummy IBL resource
    /// exists (no [`Self::set_environment`] call yet) the pass is kept but
    /// skipped at draw time — the placeholder cubemap is all black.
    pub fn set_skybox(&mut self, enabled: bool) {
        if enabled {
            if self.skybox_pass.is_none() {
                let sb = SkyboxPass::new(
                    &self.device,
                    &self.camera_buffer,
                    &self.ibl_resource.environment_view,
                    &self.ibl_processor.sampler,
                    wgpu::TextureFormat::Rgba16Float,
                    self.msaa_sample_count,
                );
                self.skybox_pass = Some(sb);
            }
        } else {
            self.skybox_pass = None;
        }
    }

    /// Recreates the PBR frame and scene bind groups with current resources.
    pub fn update_global_bind_group(&mut self) {
        self.frame_bind_group = self.pbr_pass.create_frame_bind_group(
            &self.device,
            &self.camera_buffer,
            &self.shader_debug_buffer,
            &self.ibl_resource.irradiance_view,
            &self.ibl_resource.prefiltered_view,
            &self.ibl_resource.brdf_lut_view,
            &self.ibl_processor.sampler,
            &self.blue_noise_view,
            &self.blue_noise_sampler,
        );
        self.scene_bind_group = self.pbr_pass.create_scene_bind_group(
            &self.device,
            &self.light_buffer,
            &self.shadow_view,
            &self.shadow_sampler,
            &self.ssao_blurred_view,
            &self.ssao_pbr_sampler,
            &self.cluster_manager.point_lights_buffer,
            &self.cluster_manager.metadata_buffer,
            &self.cluster_manager.light_indices_buffer,
            &self.cluster_manager.info_buffer,
            &self.csm_view,
            &self.csm_sampler,
            &self.local_shadow_atlas_view,
            &self.shadow_uniform_buffer,
            &self.vsm_pages_array_view,
            &self.vsm_sampler,
            &self.vsm_info_buffer,
            &self.ddgi_gpu.grid_buffer,
            &self.ddgi_gpu.probe_buffer,
            &self.sdf_gpu.scene_buffer,
            &self.sdf_gpu.view,
            &self.sdf_gpu.sampler,
            &self.vsm_shadow_history_view,
            &self.vsm_shadow_pbr_sampler,
        );
    }

    /// Returns the next available swap-chain texture so callers can render
    /// 3D + UI into the same surface before calling `present()` once.
    pub fn surface_texture(&self) -> Result<wgpu::SurfaceTexture, wgpu::SurfaceError> {
        self.surface
            .as_ref()
            .ok_or(wgpu::SurfaceError::Lost)?
            .get_current_texture()
    }

    /// Renders all 3D passes into `surface_view`.
    ///
    /// Does **not** call `present()` — the caller must do that after adding
    /// any additional passes (e.g. UI) to the same surface texture.
    pub fn render_frame_into(
        &mut self,
        objects: &[RenderObject],
        surface_view: &wgpu::TextureView,
    ) -> Result<(), wgpu::SurfaceError> {
        self.ddgi_gpu.update(
            &self.queue,
            1.0 / 60.0,
            -Vec3::from_array([
                self.light_uniform.dir_direction[0],
                self.light_uniform.dir_direction[1],
                self.light_uniform.dir_direction[2],
            ]),
            Vec3::from_array([
                self.light_uniform.dir_color[0],
                self.light_uniform.dir_color[1],
                self.light_uniform.dir_color[2],
            ]),
            self.light_uniform.dir_color[3],
        );

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame_encoder"),
            });

        self.ddgi_gpu.record_gpu_fill(
            &self.device,
            &self.queue,
            &mut encoder,
            objects,
            -Vec3::from_array([
                self.light_uniform.dir_direction[0],
                self.light_uniform.dir_direction[1],
                self.light_uniform.dir_direction[2],
            ]),
            Vec3::from_array([
                self.light_uniform.dir_color[0],
                self.light_uniform.dir_color[1],
                self.light_uniform.dir_color[2],
            ]),
            self.light_uniform.dir_color[3],
        );

        // 0. Update Buffers
        self.update_model_buffer(objects);

        // VSM-only policy: keep GPU light flag in sync even if callers forgot
        // to set DirectionalLight::vsm.
        if self.vsm_enabled && self.light_uniform.soft_shadows != 2 {
            self.light_uniform.soft_shadows = 2;
            let bytes = bytemuck::bytes_of(&self.light_uniform).to_vec();
            self.staging_ring.write_immediate(
                &self.queue,
                &self.light_buffer,
                0,
                &bytes,
                false,
            );
        }

        // ── TAA jitter injection ────────────────────────────────────────────
        // Jitter **projection** P (view-Z column), then VP' = P' * V.
        // Mutating combined VP.z_axis would scale the shift by world Z.
        let camera_view = Mat4::from_cols_array_2d(&self.camera_uniform.view);
        let camera_proj = Mat4::from_cols_array_2d(&self.camera_uniform.proj);
        let unjittered_vp = camera_proj * camera_view;

        let jittered_vp = if self.taa_enabled {
            let jitter =
                halton_jitter_ndc(self.taa_frame_count, self.internal_width, self.internal_height);
            let jittered_proj = apply_jitter_to_projection(camera_proj, jitter);
            let jittered_vp = jittered_proj * camera_view;

            let mut jittered_uniform = self.camera_uniform;
            jittered_uniform.proj = jittered_proj.to_cols_array_2d();
            jittered_uniform.inv_proj = jittered_proj.inverse().to_cols_array_2d();
            jittered_uniform.view_proj = jittered_vp.to_cols_array_2d();
            self.queue.write_buffer(
                &self.camera_buffer,
                0,
                bytemuck::bytes_of(&jittered_uniform),
            );
            jittered_vp
        } else {
            unjittered_vp
        };

        // ── GPU cluster light cull (optional; CPU fallback in update_clusters) ─
        if self.cluster_manager.gpu_cull {
            self.cluster_manager.record_gpu_cull(&self.device, &mut encoder, &self.queue);
        }

        if let Some(ps) = self.particle_system.as_mut() {
            if ps.gpu_sim_enabled && ps.last_alive_count > 0 {
                let mut uniforms = self.particle_sim_uniforms;
                uniforms.capacity = ps.capacity;
                uniforms.alive_count = ps.last_alive_count;
                if uniforms.delta_time <= 0.0 {
                    uniforms.delta_time = 1.0 / 60.0;
                }
                ps.record_gpu_sim(&self.device, &mut encoder, &self.queue, &uniforms);
            }
        }

        // ── VG GPU cull (optional; CPU cull done in prepare_vg_frame) ───────
        if let Some(vg) = self.vg_system.as_mut() {
            if vg.config.gpu_cull {
                vg.poll_gpu_cull_count(&self.device);
            }
            if vg.config.gpu_cull && vg.active_instance_count() > 0 {
                let cam_pos = Vec3::new(
                    self.camera_uniform.camera_pos[0],
                    self.camera_uniform.camera_pos[1],
                    self.camera_uniform.camera_pos[2],
                );
                vg.record_gpu_cull(
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    &self.camera_uniform.view_proj,
                    cam_pos,
                    &self.hiz,
                );
            } else {
                vg.upload_meshlets_if_dirty(&self.queue);
            }
        }

        // 0. Update Light Uniform with Shadow Matrix
        let mut light_u = self.pbr_pass_light_uniform();
        if self.uses_scatter_light() {
            light_u.point_light_path = 1;
        }
        // Keep screen-space VSM flag in sync (PBR branches on this).
        light_u.vsm_screen_shadows = u32::from(self.vsm_screen_shadows);
        // Compute CSM matrices using CsmState
        let light_dir = redox_math::Vec3::new(
            light_u.dir_direction[0],
            light_u.dir_direction[1],
            light_u.dir_direction[2],
        )
        .normalize();

        // Real camera matrices: the uniform now stores view / proj separately,
        // so cascades are built from the actual camera (fixes the hardcoded
        // origin-camera bug that broke directional shadows).
        let camera_view = Mat4::from_cols_array_2d(&self.camera_uniform.view);
        let camera_proj = Mat4::from_cols_array_2d(&self.camera_uniform.proj);
        
        // Update CSM state and get shadow uniform with all cascade matrices.
        // Use the live camera near/far so cascade splits match projection unprojection.
        let shadow_u = self.csm_state.update(
            camera_view,
            camera_proj,
            light_dir,
            SHADOW_SIZE,
            self.camera_near,
            self.camera_far,
        );

        // Per-cascade caster masks (conservative AABB vs cascade clip volume).
        let cascade_count = self.csm_state.config.cascade_count.min(4) as usize;
        let cascade_vis: Vec<Vec<bool>> = (0..cascade_count)
            .map(|ci| {
                let vp = Mat4::from_cols_array_2d(&shadow_u.csm_matrices[ci]);
                objects
                    .iter()
                    .map(|obj| aabb_overlaps_clip(obj.bounds, vp))
                    .collect()
            })
            .collect();

        self.shadow_uniform = shadow_u;
        self.queue.write_buffer(
            &self.shadow_uniform_buffer,
            0,
            bytemuck::bytes_of(&self.shadow_uniform),
        );

        // For backward compatibility, also update light_u.shadow_view_proj 
        // with first cascade matrix (legacy single cascade support)
        light_u.shadow_view_proj = shadow_u.csm_matrices[0];
        self.update_light_buffer(&light_u);

        let cam_eye = Vec3::new(
            self.camera_uniform.camera_pos[0],
            self.camera_uniform.camera_pos[1],
            self.camera_uniform.camera_pos[2],
        );

        // Collect VSM dirty pages *before* CSM so we can skip 4 cascade
        // depth passes once the clipmap is resident (biggest GPU win).
        let vsm_dirty: Vec<crate::shadow::vsm_clipmap::DirtyPage> = if self.vsm_enabled {
            self.vsm_clipmap.update(cam_eye, light_dir);
            let mut aabbs: Vec<redox_math::Aabb> = objects.iter().map(|o| o.bounds).collect();
            // Always request pages near the camera frustum, not only object bounds.
            let frustum_pad = self.camera_far * 0.15;
            aabbs.push(redox_math::Aabb {
                min: cam_eye - redox_math::Vec3::splat(frustum_pad),
                max: cam_eye + redox_math::Vec3::splat(frustum_pad),
            });
            let dynamic_aabbs: Vec<redox_math::Aabb> = objects
                .iter()
                .filter(|o| {
                    let ext = o.bounds.max - o.bounds.min;
                    !(ext.y < 0.6 && ext.x > 8.0 && ext.z > 8.0) // ground
                        && ext.x < 6.0 && ext.z < 6.0 && ext.y < 4.0 // small dynamic
                })
                .map(|o| o.bounds)
                .collect();
            // Debounce dynamic invalidation: only if centers moved >0.5м (prevents 4м grid thrash)
            let cur_centers: Vec<redox_math::Vec3> = dynamic_aabbs.iter().map(|a| (a.min + a.max) * 0.5).collect();
            let need_invalidate = if cur_centers.len() != self.prev_dynamic_centers.len() {
                true
            } else {
                cur_centers.iter().zip(self.prev_dynamic_centers.iter()).any(|(c, p)| (*c - *p).length() > 0.5)
            };
            if need_invalidate {
                self.prev_dynamic_centers = cur_centers;
                self.vsm_clipmap.invalidate_aabbs(&dynamic_aabbs);
            }
            let requested = self.vsm_clipmap.request_bits_covering_aabbs(&aabbs);
            let dirty = self.vsm_clipmap.requested_dirty_pages(&requested);
            self.vsm_dirty_pages_last_frame = dirty.len() as u32;
            self.vsm_warm = dirty.is_empty();
            dirty
        } else {
            Vec::new()
        };
        // VSM-only: CSM passes skipped (deleted per user request).
        let render_csm_depth = false;
        compute_main_visibility(
            &self.camera_uniform.view_proj,
            objects,
            &mut self.main_vis_scratch,
        );

        let mut mt_cmd_buffers: Vec<wgpu::CommandBuffer> = Vec::new();
        if self.mt_command_recording {
            let shadow_ctx = ShadowContext {
                device: &self.device,
                queue: &self.queue,
                shadow_pass: &self.shadow_pass,
                shadow_model_bind_group: &self.shadow_model_bind_group,
                cascade_matrix_buffers: &self.cascade_matrix_buffers,
                cascade_matrix_bgs: &self.cascade_matrix_bgs,
                csm_layer_views: &self.csm_layer_views,
                vg_shadow_pass: &self.vg_shadow_pass,
                vg_shadow_instances_bg: &self.vg_shadow_instances_bg,
                registry: &self.registry,
            };
            let prepass_ctx = PrepassContext {
                device: &self.device,
                normal_pass: &self.normal_pass,
                normal_bind_group: &self.normal_bind_group,
                normal_view: &self.normal_view,
                depth_view: &self.depth_view,
                registry: &self.registry,
            };
            let vis = self.main_vis_scratch.clone();
            let (shadow_cmd, prepass_cmd) = join(
                || {
                    record_csm_shadow_depth(
                        &shadow_ctx,
                        &shadow_u,
                        &cascade_vis,
                        objects,
                        self.vg_system.as_ref(),
                        render_csm_depth,
                    )
                },
                || record_normal_prepass(&prepass_ctx, objects, &vis),
            );
            mt_cmd_buffers.push(shadow_cmd);
            mt_cmd_buffers.push(prepass_cmd);
        }

        // Render all CSM cascades using pooled buffers/views (written, not recreated).
        if !self.mt_command_recording && render_csm_depth {
            for cascade_idx in 0..self.csm_state.config.cascade_count.min(4) {
                self.queue.write_buffer(
                    &self.cascade_matrix_buffers[cascade_idx],
                    0,
                    bytemuck::bytes_of(&shadow_u.csm_matrices[cascade_idx]),
                );

                let shadow_matrix_bg = &self.cascade_matrix_bgs[cascade_idx];
                let csm_layer_view = &self.csm_layer_views[cascade_idx];

                {
                    let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some(&format!("shadow_pass_csm{}", cascade_idx)),
                        color_attachments: &[],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &csm_layer_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.0),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });

                    shadow_pass.set_pipeline(&self.shadow_pass.pipeline);
                    shadow_pass.set_bind_group(0, &shadow_matrix_bg, &[]);
                    shadow_pass.set_bind_group(1, &self.shadow_model_bind_group, &[]);

                    for (i, obj) in objects.iter().enumerate() {
                        if !cascade_vis[cascade_idx][i] {
                            continue;
                        }
                        // Skip huge flat slabs as casters — they fill the map with
                        // receiver self-depth and drown real silhouettes.
                        let ext = obj.bounds.max - obj.bounds.min;
                        if ext.y < 0.6 && ext.x > 8.0 && ext.z > 8.0 {
                            continue;
                        }
                        if let Some(gpu_mesh) = self.registry.mesh_by_slot(obj.mesh_index) {
                            shadow_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                            shadow_pass.set_index_buffer(
                                gpu_mesh.index_buffer.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            shadow_pass.draw_indexed(
                                0..gpu_mesh.index_count,
                                0,
                                (i as u32)..(i as u32 + 1),
                            );
                        }
                    }

                    // VG meshlet shadows (same indirect commands as main view).
                    if let Some(vg) = &self.vg_system {
                        if vg.has_content() {
                            vg.draw_shadow_indirect(
                                &mut shadow_pass,
                                &self.vg_shadow_pass,
                                shadow_matrix_bg,
                                &self.vg_shadow_instances_bg,
                            );
                        }
                    }
                }
            }
        }

        // --- 1b. Point-light cube shadows: MULTI-CASTER, tiled atlas ---
        // Atlas is 8192² of 512² tiles (16×16 grid = 256 tiles = 42 casters).
        // Casters are prioritised nearest-to-camera; overflow lights render
        // unshadowed (`NO_TILE` in the GPU light struct).

        use crate::shadow::atlas_allocator::{
            ShadowAtlasAllocator, ATLAS_PX, TILE_PX, TILES_PER_AXIS, NO_TILE,
        };

        // Sort casters by distance to the camera (closest first).
        let mut caster_order: Vec<(usize, f32)> = self
            .current_point_lights
            .iter()
            .enumerate()
            .filter(|(_, l)| l.cast_shadows)
            .map(|(i, l)| (i, (l.position - cam_eye).length_squared()))
            .collect();
        caster_order
            .sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut allocator = ShadowAtlasAllocator::new();
        allocator.begin_frame();
        let mut tile_bases: Vec<(usize, u32)> = Vec::new();

        let mut local_shadow_matrices = [Mat4::IDENTITY; 6];
        let face_forward_up: [(Vec3, Vec3); 6] = [
            (Vec3::X, Vec3::Y),     // PosX
            (Vec3::NEG_X, Vec3::Y), // NegX
            (Vec3::Y, Vec3::NEG_Z), // PosY
            (Vec3::NEG_Y, Vec3::Z), // NegY
            (Vec3::Z, Vec3::Y),     // PosZ
            (Vec3::NEG_Z, Vec3::Y), // NegZ
        ];

        let mut atlas_jobs: Vec<AtlasCasterJob> = Vec::new();
        for (caster_ord, &(light_idx, _)) in caster_order.iter().enumerate() {
            let tile_base = allocator.allocate(light_idx as u64);
            if tile_base == NO_TILE {
                continue;
            }
            if caster_ord >= crate::shadow::atlas_allocator::MAX_CASTERS as usize {
                break;
            }
            tile_bases.push((light_idx, tile_base));

            let light = &self.current_point_lights[light_idx];
            let light_pos = light.position;
            let near = 0.05_f32;
            let far = light.radius.max(0.5);
            let proj =
                redox_math::Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, near, far);
            for (fi, (fwd, up)) in face_forward_up.iter().enumerate() {
                let target = light_pos + *fwd;
                let view = redox_math::Mat4::look_at_rh(light_pos, target, *up);
                local_shadow_matrices[fi] = proj * view;
            }

            let face_base = caster_ord * 6;
            for (face_i, m) in local_shadow_matrices.iter().enumerate() {
                self.queue.write_buffer(
                    &self.face_matrix_buffers[face_base + face_i],
                    0,
                    bytemuck::bytes_of(&m.to_cols_array_2d()),
                );
            }

            let face_vis: [Vec<bool>; 6] = std::array::from_fn(|face_i| {
                let frustum = redox_math::Frustum::from_view_projection(local_shadow_matrices[face_i]);
                objects.iter().map(|o| frustum.intersects_aabb(&o.bounds)).collect()
            });
            atlas_jobs.push(AtlasCasterJob {
                face_base,
                tile_base,
                face_vis,
            });
        }
        let _ = (ATLAS_PX, TILES_PER_AXIS, TILE_PX);

        // Patch every caster's GPU entry with its face matrices + tile base,
        // then refresh the clustered lights storage buffer.
        if !tile_bases.is_empty() {
            use crate::light::PointLightGpu;
            let mut gpu_lights: Vec<PointLightGpu> = self
                .current_point_lights
                .iter()
                .map(PointLightGpu::from_point_light)
                .collect();

            for &(light_idx, tile_base) in &tile_bases {
                // Recompute this light's matrices (local_shadow_matrices holds
                // only the LAST caster's values at this point).
                let light = &self.current_point_lights[light_idx];
                let near = 0.05_f32;
                let far = light.radius.max(0.5);
                let proj = redox_math::Mat4::perspective_rh(
                    std::f32::consts::FRAC_PI_2, 1.0, near, far,
                );
                for (fi, (fwd, up)) in face_forward_up.iter().enumerate() {
                    let view = redox_math::Mat4::look_at_rh(
                        light.position,
                        light.position + *fwd,
                        *up,
                    );
                    gpu_lights[light_idx].shadow_matrices[fi] = (proj * view).to_cols_array_2d();
                }
                gpu_lights[light_idx].shadow_tile_base = tile_base;
            }

            let lights_bytes = bytemuck::cast_slice::<PointLightGpu, u8>(&gpu_lights);
            self.queue.write_buffer(&self.cluster_manager.point_lights_buffer, 0, lights_bytes);
        }

        let atlas_ctx = AtlasContext {
            device: &self.device,
            shadow_pass: &self.shadow_pass,
            shadow_model_bind_group: &self.shadow_model_bind_group,
            face_matrix_bgs: &self.face_matrix_bgs,
            atlas_view: &self.local_shadow_atlas_view,
            vg_shadow_pass: &self.vg_shadow_pass,
            vg_shadow_instances_bg: &self.vg_shadow_instances_bg,
            registry: &self.registry,
        };

        // ── VSM production (directional; only when the mode is enabled) ──
        // Static-scene-optimal slice: pages render once and stay cached until
        // invalidation (light moved). Dynamic-object page tracking via
        // visibility-bits readback is a documented follow-up.
        if self.vsm_enabled {
            let dirty = vsm_dirty;

            // Upload page tables AFTER allocation so PBR sees VALID bits
            // this frame (uploading first made moving-sun frames fall back
            // to CSM, then static frames sampled empty cached pages).
            let info = self
                .vsm_clipmap
                .gpu_info(&self.smrt_config);
            self.queue.write_buffer(&self.vsm_info_buffer, 0, bytemuck::bytes_of(&info));

            // Per-page PageDrawInfo packed into the dynamic-offset uniform
            // (256-byte stride, one entry per dirty page).
            let mut pageinfo_scratch: Vec<u8> = Vec::with_capacity(dirty.len() * 256);
            for &(level, px, py, _phys) in &dirty {
                let vp = self.vsm_clipmap.page_view_proj(level as usize, px, py);
                let mut page_info = [0u8; 80];
                page_info[..64].copy_from_slice(bytemuck::bytes_of(&vp.to_cols_array_2d()));
                pageinfo_scratch.extend_from_slice(&page_info);
                pageinfo_scratch.extend_from_slice(&[0u8; 176]);
            }
            if !pageinfo_scratch.is_empty() {
                self.queue.write_buffer(&self.vsm_pageinfo_buffer, 0, &pageinfo_scratch);
            }

            let vsm_ctx = VsmContext {
                device: &self.device,
                vsm_depth_pass: &self.vsm_depth_pass,
                vsm_pageinfo_bg: &self.vsm_pageinfo_bg,
                vsm_pages_views: &self.vsm_pages_views,
                registry: &self.registry,
            };
            if self.mt_command_recording {
                let (atlas_cmd, vsm_cmd) = join(
                    || {
                        record_point_atlas_shadows(
                            &atlas_ctx,
                            &atlas_jobs,
                            objects,
                            self.vg_system.as_ref(),
                        )
                    },
                    || record_vsm_pages(&vsm_ctx, &dirty, objects),
                );
                mt_cmd_buffers.push(atlas_cmd);
                mt_cmd_buffers.push(vsm_cmd);
            } else {
                mt_cmd_buffers.push(record_point_atlas_shadows(
                    &atlas_ctx,
                    &atlas_jobs,
                    objects,
                    self.vg_system.as_ref(),
                ));
                mt_cmd_buffers.push(record_vsm_pages(&vsm_ctx, &dirty, objects));
            }
            self.vsm_clipmap.mark_pages_cached(&dirty);
        } else if self.mt_command_recording {
            mt_cmd_buffers.push(record_point_atlas_shadows(
                &atlas_ctx,
                &atlas_jobs,
                objects,
                self.vg_system.as_ref(),
            ));
        } else {
            mt_cmd_buffers.push(record_point_atlas_shadows(
                &atlas_ctx,
                &atlas_jobs,
                objects,
                self.vg_system.as_ref(),
            ));
        }


        // 2. Normal pass (normals + linear depth for SSAO)
        if !self.mt_command_recording {
            let mut normal_render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("normal_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.normal_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            normal_render_pass.set_pipeline(&self.normal_pass.pipeline);
            normal_render_pass.set_bind_group(0, &self.normal_bind_group, &[]);
            for (i, obj) in objects.iter().enumerate() {
                if !self.main_vis_scratch[i] {
                    continue;
                }
                if let Some(gpu_mesh) = self.registry.mesh_by_slot(obj.mesh_index) {
                    normal_render_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                    normal_render_pass.set_index_buffer(
                        gpu_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    normal_render_pass.draw_indexed(
                        0..gpu_mesh.index_count,
                        0,
                        (i as u32)..(i as u32 + 1),
                    );
                }
            }
        }

        // Build Hi-Z pyramid from this frame's depth, then late-cull meshlets
        // that Hi-Z of the *previous* frame rejected (1-frame disocclusion).
        self.hiz.build_from_depth(
            &self.device,
            &self.queue,
            &mut encoder,
            &self.depth_view,
        );
        if let Some(vg) = self.vg_system.as_mut() {
            if vg.config.gpu_cull && vg.active_instance_count() > 0 {
                let cam_pos = Vec3::new(
                    self.camera_uniform.camera_pos[0],
                    self.camera_uniform.camera_pos[1],
                    self.camera_uniform.camera_pos[2],
                );
                vg.record_gpu_cull_phase2(
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    &self.camera_uniform.view_proj,
                    cam_pos,
                    &self.hiz,
                );
            }
        }

        // 3–5. GTAO (optional — when disabled leave bent normal + AO at defaults)
        if self.ssao_enabled {
            let mut gtao_render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gtao_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.ssao_raw_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 1.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            gtao_render_pass.set_pipeline(&self.ssao_pass.pipeline);
            gtao_render_pass.set_bind_group(0, &self.ssao_bind_group, &[]);
            gtao_render_pass.draw(0..3, 0..1);
            drop(gtao_render_pass);

            let mut blur_render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gtao_blur_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.ssao_half_blurred_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 1.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            blur_render_pass.set_pipeline(&self.ssao_pass.blur_pipeline);
            blur_render_pass.set_bind_group(0, &self.ssao_blur_bind_group, &[]);
            blur_render_pass.draw(0..3, 0..1);
            drop(blur_render_pass);

            let mut upsample_render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gtao_upsample_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.ssao_blurred_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 1.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            upsample_render_pass.set_pipeline(&self.ssao_pass.upsample_pipeline);
            upsample_render_pass.set_bind_group(0, &self.ssao_upsample_bind_group, &[]);
            upsample_render_pass.draw(0..3, 0..1);
        } else {
            let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gtao_disabled_clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.ssao_blurred_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 1.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }

        // 5b. VSM screen-space shadow (1 Vogel ray + temporal) — now default ON.
        // PBR samples filtered R16Float instead of inline 16-ray SMRT.
        if self.vsm_enabled && self.vsm_screen_shadows {
            self.queue.write_buffer(
                &self.vsm_shadow_pass.pass_uniform_buffer,
                0,
                bytemuck::bytes_of(&VsmPassUniform {
                    frame_index: self.vsm_shadow_frame_count as u32,
                    _pad: [0; 3],
                }),
            );
            self.queue.write_buffer(
                &self.vsm_shadow_pass.temporal_uniform_buffer,
                0,
                bytemuck::bytes_of(&VsmTemporalUniform {
                    blend_alpha: self.vsm_shadow_temporal_alpha,
                    reset: u32::from(self.vsm_shadow_frame_count == 0),
                    _pad: [0.0; 2],
                }),
            );

            let mut trace_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vsm_shadow_trace"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.vsm_shadow_raw_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            trace_pass.set_pipeline(&self.vsm_shadow_pass.trace_pipeline);
            trace_pass.set_bind_group(0, &self.vsm_shadow_trace_bind_group, &[]);
            trace_pass.draw(0..3, 0..1);
            drop(trace_pass);

            let mut temporal_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vsm_shadow_temporal"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.vsm_shadow_filtered_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            temporal_pass.set_pipeline(&self.vsm_shadow_pass.temporal_pipeline);
            temporal_pass.set_bind_group(0, &self.vsm_shadow_temporal_bind_group, &[]);
            temporal_pass.draw(0..3, 0..1);
            drop(temporal_pass);

            // Ping-pong: output buffer becomes next frame's history (no GPU copy).
            std::mem::swap(&mut self.vsm_shadow_filtered_texture, &mut self.vsm_shadow_history_texture);
            std::mem::swap(&mut self.vsm_shadow_filtered_view, &mut self.vsm_shadow_history_view);
            self.vsm_shadow_temporal_bind_group = self.vsm_shadow_pass.create_temporal_bind_group(
                &self.device,
                &self.vsm_shadow_raw_view,
                &self.vsm_shadow_history_view,
                &self.normal_view,
                &self.camera_buffer,
                &self.common_sampler,
            );
            self.update_global_bind_group();
        } else {
            let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vsm_shadow_disabled_clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.vsm_shadow_filtered_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }

        // 5. Forward pass into HDR. Always clear + draw regular meshes.
        // VG vis/resolve composites on top; skipping this left HDR uninitialized
        // (full-screen white) as soon as any VG instance existed.
        let hdr_resolve_pass =
            self.hdr_geometry_resolve_pass(objects, &self.main_vis_scratch);
        let msaa_sample_count = self.msaa_sample_count;
        let debug_lines = std::mem::take(&mut self.current_debug_lines);
        if !debug_lines.is_empty() {
            self.debug_lines_pass
                .upload_lines(&self.queue, &debug_lines);
        }
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("forward_pass_hdr"),
                color_attachments: &[Some(Self::hdr_color_attachment(
                    &self.hdr_view,
                    &self.hdr_msaa_view,
                    msaa_sample_count,
                    wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.05,
                        g: 0.05,
                        b: 0.08,
                        a: 1.0,
                    }),
                    hdr_resolve_pass,
                    HdrGeometryResolvePass::Forward,
                ))],
                depth_stencil_attachment: Some(self.geometry_depth_attachment(wgpu::LoadOp::Clear(1.0))),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            // Skybox background:
            // opaque fragment overwrites it. Skipped while the dummy (black)
            // IBL placeholder is active.
            if let Some(sb) = &self.skybox_pass {
                if !self.ibl_resource.is_dummy() {
                    sb.draw(&mut render_pass);
                }
            }

            self.record_draw(&mut render_pass, objects, &self.main_vis_scratch);
            if !debug_lines.is_empty() {
                self.debug_lines_pass
                    .draw(&self.camera_bind_group, &mut render_pass);
            }
        }

        // ── 5a. VG visibility buffer + PBR resolve ───────────────────────────
        if self.vg_use_vis_path {
            let vg_has_content = self
                .vg_system
                .as_ref()
                .map(|v| v.has_content())
                .unwrap_or(false);
            if vg_has_content {
                if self.vg_bind_groups_dirty {
                    self.rebuild_vg_bind_groups();
                }
                if let Some(vg) = &self.vg_system {
                    {
                        let mut vis_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("vg_vis_pass"),
                            color_attachments: &[
                                Some(wgpu::RenderPassColorAttachment {
                                    view: &self.vg_vis_targets.id_view,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        // R32Uint clear: do NOT use f32::from_bits(0xFFFF_FFFF)
                                        // (that is NaN; many drivers clear uint as 0). wgpu casts
                                        // Color.r as u32, and VIS_ID_SENTINEL as f64 is exact.
                                        load: wgpu::LoadOp::Clear(wgpu::Color {
                                            r: VIS_ID_SENTINEL as f64,
                                            g: 0.0,
                                            b: 0.0,
                                            a: 1.0,
                                        }),
                                        store: wgpu::StoreOp::Store,
                                    },
                                }),
                                Some(wgpu::RenderPassColorAttachment {
                                    view: &self.vg_vis_targets.normal_depth_view,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                        store: wgpu::StoreOp::Store,
                                    },
                                }),
                                Some(wgpu::RenderPassColorAttachment {
                                    view: &self.vg_vis_targets.uv_view,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                        store: wgpu::StoreOp::Store,
                                    },
                                }),
                            ],
                            depth_stencil_attachment: Some(
                                wgpu::RenderPassDepthStencilAttachment {
                                    view: &self.depth_view,
                                    depth_ops: Some(wgpu::Operations {
                                        load: wgpu::LoadOp::Load,
                                        store: wgpu::StoreOp::Store,
                                    }),
                                    stencil_ops: None,
                                },
                            ),
                            timestamp_writes: None,
                            occlusion_query_set: None,
                        });
                        vg.draw_vis_indirect(
                            &mut vis_pass,
                            &self.vg_vis_pass,
                            &self.frame_bind_group,
                        );
                    }

                    {
                        let mut resolve_pass =
                            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("vg_resolve_pass"),
                                color_attachments: &[Some(Self::hdr_color_attachment(
                                    &self.hdr_view,
                                    &self.hdr_msaa_view,
                                    msaa_sample_count,
                                    wgpu::LoadOp::Load,
                                    hdr_resolve_pass,
                                    HdrGeometryResolvePass::VgResolve,
                                ))],
                                depth_stencil_attachment: None,
                                timestamp_writes: None,
                                occlusion_query_set: None,
                            });
                        self.vg_resolve_pass.draw(
                            &mut resolve_pass,
                            &self.frame_bind_group,
                            &self.scene_bind_group,
                            &self.vg_resolve_bind_group,
                        );
                    }
                }
            }
        } else if let Some(vg) = &self.vg_system {
            // Debug forward path (teal normals).
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vg_debug_forward"),
                color_attachments: &[Some(Self::hdr_color_attachment(
                    &self.hdr_view,
                    &self.hdr_msaa_view,
                        msaa_sample_count,
                        wgpu::LoadOp::Load,
                    hdr_resolve_pass,
                    HdrGeometryResolvePass::VgDebug,
                ))],
                depth_stencil_attachment: Some(self.geometry_depth_attachment(wgpu::LoadOp::Load)),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if vg.has_content() {
                vg.render(&mut render_pass);
            }
        }
        // ── 5b. Transparent pass:
        // Same HDR target and depth buffer as the forward pass, but with
        // depth writes disabled (read-only depth) so transparent surfaces
        // never occlude each other.
        if self.has_alpha_objects(objects, &self.main_vis_scratch) {
            let mut transparent_render_pass =
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("transparent_pass"),
                    color_attachments: &[Some(Self::hdr_color_attachment(
                        &self.hdr_view,
                        &self.hdr_msaa_view,
                    msaa_sample_count,
                    wgpu::LoadOp::Load,
                    hdr_resolve_pass,
                    HdrGeometryResolvePass::Transparent,
                    ))],
                    depth_stencil_attachment: Some(self.geometry_depth_attachment(wgpu::LoadOp::Load)),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
            self.record_draw_transparent(&mut transparent_render_pass, objects, &self.main_vis_scratch);
        }

        // ── 5c. Smart Resolve (Sharp AA): luma-weighted MSAA resolve → hdr_view ─
        if self.aa_sharp && self.msaa_sample_count > 1 {
            self.msaa_resolve_pass.record(
                &mut encoder,
                &self.msaa_resolve_bg,
                &self.hdr_view,
            );
        }

        // ── 5d. GPU particles (billboards + reactive mask) ─────────────────
        if self.reactive_view.is_some() {
            if let Some(reactive) = &self.reactive_view {
                let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("reactive_clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: reactive,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
            }
        }
        if let Some(ps) = &self.particle_system {
            if ps.last_alive_count > 0 || ps.gpu_sim_enabled {
                let reactive = self.reactive_view.as_ref();
                let mut particle_pass =
                    ps.begin_draw_pass(&mut encoder, &self.hdr_view, &self.depth_view, reactive);
                ps.record_draw(&mut particle_pass, reactive);
            }
        }

        // ── ScatterLight (HDR, pre-TAA) ─────────────────────────────────────
        self.record_scatter_light_pass(&mut encoder);

        // ── Tone-mapping uniform (shared by both call sites below) ─────────
        self.queue.write_buffer(
            &self.tonemap_uniform_buffer,
            0,
            bytemuck::bytes_of(&TonemapUniform {
                exposure: self.exposure,
                sharpen_enabled: if self.taa_enabled { 1 } else { 0 },
                bloom_enabled: if self.bloom_enabled { 1 } else { 0 },
                rcas_sharpness: if self.taa_enabled {
                    self.rcas_sharpness
                } else {
                    0.0
                },
                output_size: [self.config.width as f32, self.config.height as f32],
            }),
        );

        // ── TAA: velocity + accumulation passes ─────────────────────────────
        if self.taa_enabled {
            // --- 6a. Update velocity uniform (same P'*V as rasterization) ---
            let inv_jittered_vp = jittered_vp.inverse();

            let jitter = halton_jitter_ndc(
                self.taa_frame_count,
                self.internal_width,
                self.internal_height,
            );
            let vel_uniform = VelocityUniform {
                inv_curr_vp: inv_jittered_vp.to_cols_array_2d(),
                prev_vp: self.taa_prev_vp_unjittered.to_cols_array_2d(),
                screen_size: [self.internal_width as f32, self.internal_height as f32],
                jitter_uv: [jitter[0] * 0.5, -jitter[1] * 0.5],
            };
            self.queue.write_buffer(
                &self.velocity_pass.uniform_buffer,
                0,
                bytemuck::bytes_of(&vel_uniform),
            );


            // --- 6b. Velocity pass ---
            {
                let mut vel_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("velocity_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.taa_velocity_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.0,
                                g: 0.0,
                                b: 0.0,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                vel_pass.set_pipeline(&self.velocity_pass.pipeline);
                vel_pass.set_bind_group(0, &self.velocity_bg, &[]);
                vel_pass.draw(0..3, 0..1);
            }

            // --- 6c. Update TAA uniform ---
            let is_first_frame = self.taa_frame_count == 0;
            let jitter = halton_jitter_ndc(
                self.taa_frame_count,
                self.internal_width,
                self.internal_height,
            );
            let taa_uniform = TaaUniform {
                output_size: [self.config.width as f32, self.config.height as f32],
                input_size: [self.internal_width as f32, self.internal_height as f32],
                blend_alpha: self.taa_blend_alpha,
                reset: if is_first_frame { 1 } else { 0 },
                jitter_uv: [jitter[0] * 0.5, -jitter[1] * 0.5],
            };
            self.queue.write_buffer(
                &self.taa_pass.uniform_buffer,
                0,
                bytemuck::bytes_of(&taa_uniform),
            );

            // Ping-pong: read from history_read_idx, write to the other.
            let history_read_idx = self.taa_history_read_idx;
            let history_write_idx = 1 - history_read_idx;
            // Bind groups are prebuilt per history slot and rebuilt on resize.
            let taa_bg = &self.taa_history_bgs[history_read_idx];

            // --- 6d. TAA accumulation pass → write history ---
            {
                let mut taa_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("taa_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.taa_history_view[history_write_idx],
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                taa_pass.set_pipeline(&self.taa_pass.pipeline);
                taa_pass.set_bind_group(0, &taa_bg, &[]);
                taa_pass.draw(0..3, 0..1);
            }

            // --- 6e. Bloom from stabilized TAA output, then tone map ---
            let taa_out_view = &self.taa_history_view[history_write_idx];
            self.record_bloom_chain(&mut encoder, taa_out_view);

            // Prebuilt bind group over the freshly written history slot.
            let taa_tone_bg = &self.taa_tone_bgs[history_write_idx];
            {
                let mut post_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("tone_mapping_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &surface_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                post_pass.set_pipeline(&self.tone_mapping_pipeline);
                post_pass.set_bind_group(0, &taa_tone_bg, &[]);
                post_pass.draw(0..3, 0..1);
            }

            // --- 6f. Advance ping-pong and frame counter ---
            self.taa_history_read_idx = history_write_idx;
            self.taa_prev_vp_unjittered = unjittered_vp;
            self.taa_frame_count += 1;
        } else if self.aa_sharp {
            // ── Sharp AA path: ACES→LDR → SMAA → RCAS→surface ────────────
            self.record_bloom_chain(&mut encoder, &self.hdr_view);

            // 1) ACES tonemap HDR+bloom → ldr_view (Rgba8Unorm, no RCAS)
            self.queue.write_buffer(
                &self.tonemap_uniform_buffer,
                0,
                bytemuck::bytes_of(&TonemapUniform {
                    exposure: self.exposure,
                    sharpen_enabled: 0,
                    bloom_enabled: if self.bloom_enabled { 1 } else { 0 },
                    rcas_sharpness: 0.0,
                    output_size: [self.config.width as f32, self.config.height as f32],
                }),
            );
            {
                let mut ldr_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("sharp_tonemap_ldr"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.ldr_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                ldr_pass.set_pipeline(&self.tone_mapping_ldr_pipeline);
                ldr_pass.set_bind_group(0, &self.tone_mapping_ldr_bg, &[]);
                ldr_pass.draw(0..3, 0..1);
            }

            // 2) SMAA 1× on LDR
            self.smaa_pass.record(
                &self.device,
                &self.queue,
                &mut encoder,
                &self.ldr_view,
                &self.smaa_out_view,
            );

            // 3) RCAS sharpen → surface (fs_present_rcas: no ACES, just RCAS)
            self.queue.write_buffer(
                &self.tonemap_rcas_uniform_buffer,
                0,
                bytemuck::bytes_of(&TonemapUniform {
                    exposure: 1.0,
                    sharpen_enabled: 1,
                    bloom_enabled: 0,
                    rcas_sharpness: self.rcas_sharpness,
                    output_size: [self.config.width as f32, self.config.height as f32],
                }),
            );
            {
                let mut rcas_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("sharp_rcas_present"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &surface_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                rcas_pass.set_pipeline(&self.rcas_present_pipeline);
                rcas_pass.set_bind_group(0, &self.tone_mapping_rcas_bg, &[]);
                rcas_pass.draw(0..3, 0..1);
            }
        } else {
            // ── Standard path (no TAA, no Sharp): bloom then tone map from HDR
            self.record_bloom_chain(&mut encoder, &self.hdr_view);
            let mut post_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("tone_mapping_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            post_pass.set_pipeline(&self.tone_mapping_pipeline);
            post_pass.set_bind_group(0, &self.tone_mapping_bind_group, &[]);
            post_pass.draw(0..3, 0..1);
        }

        // Copy current depth → prev depth for next frame's TAA disocclusion test.
        if self.taa_enabled {
            encoder.copy_texture_to_texture(
                wgpu::ImageCopyTexture {
                    texture: &self.depth_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::DepthOnly,
                },
                wgpu::ImageCopyTexture {
                    texture: &self.prev_depth_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::DepthOnly,
                },
                wgpu::Extent3d {
                    width: self.internal_width,
                    height: self.internal_height,
                    depth_or_array_layers: 1,
                },
            );
        }

        if let Some(vg) = self.vg_system.as_mut() {
            if vg.config.gpu_cull {
                vg.schedule_gpu_cull_readback(&mut encoder);
            }
        }

        // Feed next frame's motion reprojection (VSM temporal, future TAA-adjacent passes).
        self.camera_uniform.snapshot_prev_view_proj();
        if self.vsm_enabled {
            self.vsm_shadow_frame_count = self.vsm_shadow_frame_count.saturating_add(1);
        }

        self.queue.submit(
            mt_cmd_buffers
                .into_iter()
                .chain(std::iter::once(encoder.finish())),
        );
        self.staging_ring.end_frame();
        Ok(())
    }

    /// Convenience wrapper: acquires a surface texture, renders into it, then
    /// presents it. Use `render_frame_into` + manual `present()` when you need
    /// to add additional passes (e.g. egui) before presentation.
    /// Render one frame from an explicit [`FrameView`] (preferred API).
    pub fn render_frame_view(
        &mut self,
        fv: &FrameView<'_>,
        surface_view: &wgpu::TextureView,
    ) -> Result<(), wgpu::SurfaceError> {
        self.camera_near = fv.camera.near;
        self.camera_far = fv.camera.far;
        let light_u = fv.lights.to_legacy_uniform();
        self.update_light_buffer(&light_u);
        self.current_point_lights = fv.lights.points.clone();
        self.update_cluster_lights(&fv.lights.points, fv.camera);
        self.render_frame_into(fv.objects, surface_view)
    }

    pub fn render_frame(&mut self, objects: &[RenderObject]) -> Result<(), wgpu::SurfaceError> {
        let output = self
            .surface
            .as_ref()
            .ok_or(wgpu::SurfaceError::Lost)?
            .get_current_texture()?;
        let surface_view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.render_frame_into(objects, &surface_view)?;
        output.present();
        Ok(())
    }

    /// Writes the current camera uniform to the GPU.
    pub fn update_camera_buffer(&mut self) {
        let bytes = bytemuck::bytes_of(&self.camera_uniform).to_vec();
        self.staging_ring.write_immediate(
            &self.queue,
            &self.camera_buffer,
            0,
            &bytes,
            false,
        );
    }

    /// Returns the current light uniform.
    pub fn pbr_pass_light_uniform(&self) -> LightUniform {
        self.light_uniform
    }

    /// Writes a new light uniform to the GPU.
    ///
    /// When [`Self::vsm_enabled`] is true, forces `soft_shadows = 2` (VSM path)
    /// so the PBR shader never falls through to classic CSM.
    pub fn update_light_buffer(&mut self, light: &LightUniform) {
        let mut light = *light;
        if self.vsm_enabled {
            light.soft_shadows = 2;
        }
        self.light_uniform = light;
        let bytes = bytemuck::bytes_of(&light).to_vec();
        self.staging_ring.write_immediate(
            &self.queue,
            &self.light_buffer,
            0,
            &bytes,
            false,
        );
    }

    /// Writes the shader debug uniform (debug_viz_mode) to the GPU.
    pub fn update_shader_debug_buffer(&mut self, mode: ShaderDebugVizMode) {
        let mut u = ShaderDebugUniform::default();
        u.debug_viz_mode = mode as u32;
        u.frame_index = self.taa_frame_count as u32;
        u.ibl_enabled = u32::from(!self.ibl_resource.is_dummy());
        u.gtao_enabled = u32::from(self.ssao_enabled);
        let bytes = bytemuck::bytes_of(&u).to_vec();
        self.staging_ring.write_immediate(
            &self.queue,
            &self.shader_debug_buffer,
            0,
            &bytes,
            false,
        );
    }

    pub fn update_model_buffer(&mut self, objects: &[RenderObject]) {
        self.model_uniform_scratch.clear();
        self.model_uniform_scratch.reserve(objects.len());
        for obj in objects {
            let m3 = Mat3::from_cols(
                obj.model_matrix.x_axis.truncate(),
                obj.model_matrix.y_axis.truncate(),
                obj.model_matrix.z_axis.truncate(),
            );
            let nt = m3.inverse().transpose();
            let (bounds_min, bounds_extent) = self
                .registry
                .mesh_by_slot(obj.mesh_index)
                .map(|m| bounds_vectors(&m.local_bounds))
                .unwrap_or(([0.0; 4], [1.0, 1.0, 1.0, 0.0]));
            self.model_uniform_scratch.push(ModelUniform {
                model: obj.model_matrix.to_cols_array_2d(),
                normal_matrix: [
                    nt.x_axis.extend(0.0).to_array(),
                    nt.y_axis.extend(0.0).to_array(),
                    nt.z_axis.extend(0.0).to_array(),
                ],
                color: obj.color,
                bounds_min,
                bounds_extent,
            });
        }
        let bytes = bytemuck::cast_slice(&self.model_uniform_scratch).to_vec();
        self.staging_ring.write_immediate(
            &self.queue,
            &self.model_buffer,
            0,
            &bytes,
            false,
        );
    }

    /// Updates cluster light assignments. Call this when lights change or camera is updated.
    ///
    /// Must be called **after** `update_camera_buffer()` so that `camera_uniform.camera_pos`
    /// already holds the correct world-space camera position for this frame.
    pub fn update_cluster_lights(&mut self, lights: &[crate::light::PointLight], camera: &Camera) {
        self.current_point_lights = lights.to_vec();
        let cp = self.camera_uniform.camera_pos;
        let cam_pos = redox_math::Vec3::new(cp[0], cp[1], cp[2]);
        self.cluster_manager.update_clusters(
            lights,
            camera,
            cam_pos,
            Mat4::from_cols_array_2d(&self.camera_uniform.view_proj),
            &self.device,
            &self.queue,
        );
    }

    /// Returns cluster manager for GPU cluster buffer bindings (if needed).
    pub fn cluster_manager(&self) -> &ClusterManager {
        &self.cluster_manager
    }

    // ── Virtual Geometry ──────────────────────────────────────────────────────

    /// Initialise the Virtual Geometry subsystem with the given config.
    ///
    /// Call once after [`RenderContext::new`].  After this, `vg_system` is
    /// `Some` and VG assets / instances can be registered.
    pub fn init_virtual_geometry(&mut self, config: VGConfig) {
        // Use HDR format since VG objects are rendered into the HDR buffer in the forward pass
        let hdr_fmt = wgpu::TextureFormat::Rgba16Float;
        let depth_fmt = crate::pass::forward::DEPTH_FORMAT;
        let vg = VGSystem::new(&self.device, config, &self.camera_buffer, hdr_fmt, depth_fmt);
        self.vg_system = Some(vg);
        self.rebuild_vg_bind_groups();
    }

    /// Register a mesh as a VG asset, returning its [`VGAssetId`].
    ///
    /// Requires [`init_virtual_geometry`] to have been called first.
    pub fn register_vg_mesh(
        &mut self,
        mesh: &crate::mesh::Mesh,
        material_index: u32,
    ) -> Option<VGAssetId> {
        self.vg_system.as_mut()?.register_mesh(mesh, material_index, &self.queue)
    }

    /// Prepare the VG system for this frame and synchronise transforms from ECS.
    ///
    /// Must be called each frame before [`render_frame`].
    pub fn prepare_vg_frame(&mut self, world: &mut redox_ecs::world::World) {
        let vg = match self.vg_system.as_mut() {
            Some(v) => v,
            None => return,
        };

        // Phase 1: collect VG entities via query (not all_entities scan).
        type Entity = redox_ecs::Entity;
        self.vg_sync_scratch.clear();
        for (entity, vm, transform) in world
            .query::<(Entity, &VirtualMesh, &Transform)>()
            .iter(world)
        {
            self.vg_sync_scratch.push((
                entity,
                transform.matrix().to_cols_array_2d(),
                vm.instance_handle,
                vm.asset_id,
            ));
        }

        // Phase 2: apply updates and spawn new instances, then write back handles
        let mut new_handles: Vec<(Entity, crate::virtual_geometry::VGInstanceHandle)> = Vec::new();
        for (entity, mat, handle_opt, asset_id) in &self.vg_sync_scratch {
            if let Some(handle) = *handle_opt {
                vg.update_instance_transform(handle, *mat);
            } else if let Some(handle) = vg.spawn_instance(*asset_id, *mat) {
                new_handles.push((*entity, handle));
            }
        }
        // Phase 3: write back newly assigned instance handles
        for (entity, handle) in new_handles {
            if let Some(vm_mut) = world.get_component_mut::<VirtualMesh>(entity) {
                vm_mut.instance_handle = Some(handle);
            }
        }

        // Run CPU culling and upload draw commands
        let vp = self.camera_uniform.view_proj;
        let cam_pos = redox_math::Vec3::from_array([
            self.camera_uniform.camera_pos[0],
            self.camera_uniform.camera_pos[1],
            self.camera_uniform.camera_pos[2],
        ]);
        // Recover vertical FOV from projection matrix (perspective RH [0,1]).
        let fov_y = 2.0 * (1.0 / self.camera_uniform.proj[1][1]).atan();
        vg.prepare_frame(&self.queue, &vp, cam_pos, fov_y);
    }
}

/// Helper to create a render target texture for normal or GTAO pass.
fn create_normal_ssao_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    label: &str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Pick the best HDR MSAA sample count supported by the adapter (prefer ×4, then ×2).
///
/// Used by [`crate::Renderer`] and editor offscreen hosts that share the Sharp AA path.
pub fn pick_hdr_msaa_sample_count(
    adapter: &wgpu::Adapter,
    format: wgpu::TextureFormat,
    want_msaa: bool,
) -> u32 {
    if !want_msaa {
        return 1;
    }
    let flags = adapter.get_texture_format_features(format).flags;
    for count in [4u32, 2] {
        if flags.sample_count_supported(count) {
            return count;
        }
    }
    1
}

/// Rgba8Unorm intermediate for Sharp AA (LDR tonemap output or SMAA result).
fn create_sharp_intermediate(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    label: &str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Previous-frame depth buffer for TAA disocclusion (Depth32Float copy).
fn create_prev_depth_texture(
    device: &wgpu::Device,
    _queue: &wgpu::Queue,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("prev_depth_texture"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: crate::pass::forward::DEPTH_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Helper to create HDR texture, view and sampler.
fn create_hdr_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Sampler) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hdr_texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("hdr_sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });
    (texture, view, sampler)
}

/// Multisampled HDR colour target for geometry passes (TXAA-style MSAA input).
/// Includes TEXTURE_BINDING so the Smart Resolve pass can read individual samples.
fn create_hdr_msaa_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    sample_count: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hdr_msaa_texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Multisampled depth target matching HDR MSAA sample count.
fn create_depth_msaa_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    sample_count: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth_msaa_texture"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format: crate::pass::forward::DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn create_bloom_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    label: &str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

fn create_bloom_mip_chain(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> (
    [wgpu::Texture; crate::post::bloom::BLOOM_MIP_COUNT],
    [wgpu::TextureView; crate::post::bloom::BLOOM_MIP_COUNT],
) {
    use crate::post::bloom::BLOOM_MIP_COUNT;
    let mut textures = Vec::with_capacity(BLOOM_MIP_COUNT);
    let mut views = Vec::with_capacity(BLOOM_MIP_COUNT);
    let mut w = width;
    let mut h = height;
    for i in 0..BLOOM_MIP_COUNT {
        w = (w / 2).max(1);
        h = (h / 2).max(1);
        let (tex, view) = create_bloom_texture(device, w, h, format, &format!("bloom_mip{i}"));
        textures.push(tex);
        views.push(view);
    }
    (
        textures.try_into().unwrap(),
        views.try_into().unwrap(),
    )
}

fn create_bloom_downsample_bgs(
    bloom_pass: &crate::post::bloom::BloomPass,
    device: &wgpu::Device,
    hdr_view: &wgpu::TextureView,
    mip_views: &[wgpu::TextureView; crate::post::bloom::BLOOM_MIP_COUNT],
    sampler: &wgpu::Sampler,
) -> [wgpu::BindGroup; crate::post::bloom::BLOOM_MIP_COUNT] {
    use crate::post::bloom::BLOOM_MIP_COUNT;
    let mut bgs = Vec::with_capacity(BLOOM_MIP_COUNT);
    for i in 0..BLOOM_MIP_COUNT {
        let src = if i == 0 { hdr_view } else { &mip_views[i - 1] };
        bgs.push(bloom_pass.create_bind_group(device, src, sampler));
    }
    bgs.try_into().unwrap()
}

fn create_bloom_upsample_bgs(
    bloom_pass: &crate::post::bloom::BloomPass,
    device: &wgpu::Device,
    mip_views: &[wgpu::TextureView; crate::post::bloom::BLOOM_MIP_COUNT],
    sampler: &wgpu::Sampler,
) -> [wgpu::BindGroup; crate::post::bloom::BLOOM_MIP_COUNT - 1] {
    use crate::post::bloom::BLOOM_MIP_COUNT;
    let mut bgs = Vec::with_capacity(BLOOM_MIP_COUNT - 1);
    for i in 0..BLOOM_MIP_COUNT - 1 {
        // Upsample reads the coarser mip (i+1) into mip i.
        bgs.push(bloom_pass.create_bind_group(device, &mip_views[i + 1], sampler));
    }
    bgs.try_into().unwrap()
}

/// Conservative overlap test: world AABB vs ortho clip cube `[-1,1]×[-1,1]×[0,1]`.
fn aabb_overlaps_clip(bounds: Aabb, view_proj: Mat4) -> bool {
    let b = bounds;
    let corners = [
        Vec3::new(b.min.x, b.min.y, b.min.z),
        Vec3::new(b.max.x, b.min.y, b.min.z),
        Vec3::new(b.min.x, b.max.y, b.min.z),
        Vec3::new(b.max.x, b.max.y, b.min.z),
        Vec3::new(b.min.x, b.min.y, b.max.z),
        Vec3::new(b.max.x, b.min.y, b.max.z),
        Vec3::new(b.min.x, b.max.y, b.max.z),
        Vec3::new(b.max.x, b.max.y, b.max.z),
    ];
    let mut min_c = Vec3::splat(f32::INFINITY);
    let mut max_c = Vec3::splat(f32::NEG_INFINITY);
    for c in corners {
        let p = view_proj.transform_point3(c);
        min_c = min_c.min(p);
        max_c = max_c.max(p);
    }
    !(max_c.x < -1.0
        || min_c.x > 1.0
        || max_c.y < -1.0
        || min_c.y > 1.0
        || max_c.z < 0.0
        || min_c.z > 1.0)
}
