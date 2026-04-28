//! GPU context: owns the `wgpu` instance, device, queue, and surface.
//!
//! Also acts as a simple resource store for meshes, materials, and textures
//! in the MVP (these will move to a dedicated asset manager later).

use bytemuck;
use std::collections::HashMap;
use std::sync::Arc;
use winit::window::Window;

use redox_asset::{AssetId, Handle};

use crate::asset_types::{MaterialData, MeshData, TextureData};
use crate::camera::CameraUniform;
use crate::cluster_manager::ClusterManager;
use crate::light::{LightUniform, ShaderDebugUniform};

/// Debug visualization mode for the PBR shader (binding 21).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum ShaderDebugVizMode {
    #[default]
    None = 0,
    ClusterIndex = 1,
    ShadowValue = 2,
}
use crate::material::Material;
use crate::mesh::Mesh;
use crate::pass::debug_lines::DebugLinesPass;
use crate::pass::forward::{ForwardPass, ModelUniform, create_depth_texture};
use crate::pass::normal::NormalPass;
use crate::pass::pbr::PbrPass;
use crate::pass::shadow::{SHADOW_FORMAT, SHADOW_SIZE, ShadowPass};
use crate::shadow::csm::{CsmConfig, CsmState, ShadowUniform};
use crate::virtual_geometry::runtime::{VGConfig, VGSystem};
use crate::virtual_geometry::{VGAssetId, VirtualMesh};
use crate::pass::taa::{
    TaaPass, TaaUniform,
    create_history_texture, create_velocity_texture,
    halton_jitter_ndc,
};
use crate::pass::velocity::{VelocityPass, VelocityUniform};
use redox_math::{Mat4, Vec3, look_at};
use crate::post::ssao::SSAOPass;
use crate::post::tone_mapping::create_tone_mapping_pipeline;
use crate::resource::buffer;
use crate::resource::ibl::{IBLProcessor, IBLResource};
use crate::resource::texture::Texture;
use crate::systems::RenderObject;
use crate::camera::Camera;

/// GPU-side mesh data (vertex + index buffers).
pub struct GpuMesh {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
}

/// The central rendering context.
///
/// Owns all `wgpu` state and provides methods to upload resources and render
/// a frame.
pub struct RenderContext {
    // --- wgpu core ---
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    /// Internal render scale for TAAU (render at lower res, upscale temporally).
    pub render_scale: f32,
    /// Internal render width in pixels.
    pub internal_width: u32,
    /// Internal render height in pixels.
    pub internal_height: u32,

    // --- Render pass ---
    pub forward_pass: ForwardPass,
    pub pbr_pass: PbrPass,
    pub shadow_pass: ShadowPass,
    pub normal_pass: NormalPass,
    pub ssao_pass: SSAOPass,

    // --- Global Uniforms ---
    pub camera_buffer: wgpu::Buffer,
    pub camera_uniform: CameraUniform,

    pub light_uniform: LightUniform,
    pub light_buffer: wgpu::Buffer,
    pub shader_debug_buffer: wgpu::Buffer,

    pub global_bind_group: wgpu::BindGroup,

    // --- HDR & Post-processing ---
    pub hdr_texture: wgpu::Texture,
    pub hdr_view: wgpu::TextureView,
    pub tone_mapping_pipeline: wgpu::RenderPipeline,
    pub tone_mapping_bind_group: wgpu::BindGroup,
    pub hdr_sampler: wgpu::Sampler,

    // --- Normal pass + SSAO ---
    pub normal_texture: wgpu::Texture,
    pub normal_view: wgpu::TextureView,
    pub normal_bind_group: wgpu::BindGroup,
    pub ssao_raw_texture: wgpu::Texture,
    pub ssao_raw_view: wgpu::TextureView,
    pub ssao_blurred_texture: wgpu::Texture,
    pub ssao_blurred_view: wgpu::TextureView,
    pub ssao_bind_group: wgpu::BindGroup,
    pub ssao_blur_bind_group: wgpu::BindGroup,
    /// Sampler for sampling SSAO texture in PBR (Nearest — matches filterable: false / R16Float).
    pub ssao_pbr_sampler: wgpu::Sampler,

    // --- Depth ---
    #[allow(dead_code)]
    pub depth_texture: wgpu::Texture,
    pub depth_view: wgpu::TextureView,

    pub shadow_view: wgpu::TextureView,
    pub shadow_sampler: wgpu::Sampler,
    pub csm_texture: wgpu::Texture,
    pub csm_view: wgpu::TextureView,
    pub csm_sampler: wgpu::Sampler,
    pub local_shadow_atlas: wgpu::Texture,
    pub local_shadow_atlas_view: wgpu::TextureView,
    pub shadow_uniform: ShadowUniform,
    pub shadow_uniform_buffer: wgpu::Buffer,

    // --- Resource storage (MVP) ---
    pub meshes: Vec<GpuMesh>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
    pub texture_bind_groups: Vec<wgpu::BindGroup>,
    pub fallback_texture_bg: wgpu::BindGroup,

    pub fallback_normal_view: wgpu::TextureView,
    pub fallback_mr_view: wgpu::TextureView,
    pub common_sampler: wgpu::Sampler,

    pub material_bind_groups: Vec<wgpu::BindGroup>,
    pub material_uniform_buffers: Vec<wgpu::Buffer>,

    /// Per-object model-matrix buffer (reused each frame).
    model_buffer: wgpu::Buffer,
    model_bind_group: wgpu::BindGroup,
    pub shadow_model_bind_group: wgpu::BindGroup,

    /// Map from asset handle id to index in `meshes` (for handle-based lookup).
    handle_to_mesh_index: HashMap<AssetId, usize>,
    /// Map from asset handle id to index in `textures` (for handle-based lookup).
    handle_to_texture_index: HashMap<AssetId, usize>,
    /// Map from asset handle id to index in `materials` (for handle-based lookup).
    handle_to_material_index: HashMap<AssetId, usize>,

    // --- Clustered Forward Rendering ---
    pub cluster_manager: ClusterManager,

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
    // --- CSM state ---
    pub csm_state: CsmState,

    // --- Virtual Geometry (Nanite-like) ---
    pub vg_system: Option<VGSystem>,

    /// Point lights for the current frame (shared between update_cluster_lights and render_frame_into).
    pub current_point_lights: Vec<crate::light::PointLight>,
}

impl RenderContext {
    /// Initialises the entire rendering context.
    ///
    /// This is an `async` function because `wgpu` adapter and device requests
    /// are asynchronous. Wrap with `pollster::block_on` when calling from
    /// synchronous code.
    pub async fn new(window: Arc<Window>) -> Self {
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
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("redox_device"),
                    required_features: wgpu::Features::empty(),
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
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let hdr_format = wgpu::TextureFormat::Rgba16Float;

        // --- Internal resolution for TAAU ---
        // Default quality: render above output resolution, then TAA resolve.
        // This behaves like SSAA/temporal supersampling and reduces stair-steps without adding blur.
        let render_scale = 1.70_f32;
        let internal_width = ((config.width as f32) * render_scale).round().max(1.0) as u32;
        let internal_height = ((config.height as f32) * render_scale).round().max(1.0) as u32;
        // Lower = more history (less aliasing / shimmer), higher = more responsive.
        // Tuned for default supersampling scale.
        let taa_blend_alpha = 0.05_f32;

        // --- Render passes ---
        let forward_pass = ForwardPass::new(&device, hdr_format);
        let pbr_pass = PbrPass::new(&device, hdr_format);
        let shadow_pass = ShadowPass::new(&device);
        let normal_pass = NormalPass::new(&device, wgpu::TextureFormat::Rgba16Float);
        let ssao_pass = SSAOPass::new(&device, &queue);
        let debug_lines_pass = DebugLinesPass::new(&device, &forward_pass.camera_bgl);

        // --- Camera uniform ---
        let camera_uniform = CameraUniform::default();
        let camera_buffer = buffer::create_uniform_buffer(
            &device,
            "camera_uniform",
            bytemuck::bytes_of(&camera_uniform),
        );
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera_bind_group"),
            layout: &forward_pass.camera_bgl,
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
                width: 4096,
                height: 4096,
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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let hdr_view = hdr_texture.create_view(&wgpu::TextureViewDescriptor::default());

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

        let tone_mapping_pipeline = create_tone_mapping_pipeline(&device, surface_format);
        let tone_mapping_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tone_mapping_bind_group"),
            layout: &tone_mapping_pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&hdr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&hdr_sampler),
                },
            ],
        });

        // --- Fallback textures ---
        let fallback_tex = Texture::white_1x1(&device, &queue);
        let fallback_texture_bg = forward_pass.create_texture_bind_group(&device, &fallback_tex);

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

        let mut textures = Vec::new();
        let mut texture_bind_groups = Vec::new();

        // Push fallback as index 0
        texture_bind_groups.push(forward_pass.create_texture_bind_group(&device, &fallback_tex));
        textures.push(fallback_tex);

        // --- Per-object model buffer (Storage) ---
        let model_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("model_storage_buffer"),
            size: (std::mem::size_of::<ModelUniform>() * 10000) as u64, // Support up to 10,000 objects
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let model_bind_group = forward_pass.create_model_bind_group(&device, &model_buffer);
        let shadow_model_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow_model_bg"),
            layout: &shadow_pass.model_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: model_buffer.as_entire_binding(),
            }],
        });

        // --- Normal + SSAO textures and bind groups (internal) ---
        let (normal_texture, normal_view) = create_normal_ssao_texture(
            &device,
            internal_width,
            internal_height,
            wgpu::TextureFormat::Rgba16Float,
            "normal",
        );
        let (ssao_raw_texture, ssao_raw_view) = create_normal_ssao_texture(
            &device,
            internal_width,
            internal_height,
            wgpu::TextureFormat::R16Float,
            "ssao_raw",
        );
        let (ssao_blurred_texture, ssao_blurred_view) = create_normal_ssao_texture(
            &device,
            internal_width,
            internal_height,
            wgpu::TextureFormat::R16Float,
            "ssao_blurred",
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
            label: Some("ssao_pbr_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
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
            label: Some("ssao_bind_group"),
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
                    resource: ssao_pass.kernel_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: camera_buffer.as_entire_binding(),
                },
            ],
        });

        let ssao_blur_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ssao_blur_bind_group"),
            layout: &ssao_pass.blur_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&ssao_raw_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&common_sampler),
                },
            ],
        });

        // --- TAA infrastructure ---
        let taa_pass = TaaPass::new(&device, hdr_format);
        let velocity_pass = VelocityPass::new(&device);

        let (taa_history_tex0, taa_history_view0) =
            create_history_texture(&device, config.width, config.height, "taa_history_0");
        let (taa_history_tex1, taa_history_view1) =
            create_history_texture(&device, config.width, config.height, "taa_history_1");
        let (taa_velocity_tex, taa_velocity_view) =
            create_velocity_texture(&device, internal_width, internal_height);

        // --- IBL ---
        let ibl_processor = IBLProcessor::new(&device);
        let ibl_resource = IBLResource::dummy(&device);

        // --- Clustered Forward Rendering ---
        let dummy_camera = Camera::new(std::f32::consts::PI / 4.0, 16.0 / 9.0, 0.1, 1000.0);
        let cluster_manager = ClusterManager::new(internal_width, internal_height, &dummy_camera, &device, &queue);

        let csm_state = CsmState::new(CsmConfig {
            cascade_count: 4,
            shadow_map_resolution: SHADOW_SIZE,
            near: 0.1,
            far: 1000.0,
        });

        let global_bind_group = pbr_pass.create_global_bind_group(
            &device,
            &camera_buffer,
            &light_buffer,
            &shadow_view,
            &shadow_sampler,
            &ibl_resource.irradiance_view,
            &ibl_resource.prefiltered_view,
            &ibl_resource.brdf_lut_view,
            &ssao_blurred_view,
            &ssao_pbr_sampler,
            &ibl_processor.sampler,
            &cluster_manager.point_lights_buffer,
            &cluster_manager.metadata_buffer,
            &cluster_manager.light_indices_buffer,
            &cluster_manager.info_buffer,
            &csm_view,
            &csm_sampler,
            &local_shadow_atlas_view,
            &shadow_uniform_buffer,
            &shader_debug_buffer,
        );

        let vg_system = Some(VGSystem::new(
            &device,
            VGConfig::default(),
            &camera_buffer,
            hdr_format,
            crate::pass::forward::DEPTH_FORMAT,
        ));

        Self {
            device,
            queue,
            surface,
            config,
            render_scale,
            internal_width,
            internal_height,
            forward_pass,
            pbr_pass,
            shadow_pass,
            normal_pass,
            ssao_pass,
            camera_buffer,
            camera_uniform,
            light_uniform,
            light_buffer,
            shader_debug_buffer,
            global_bind_group,
            hdr_texture,
            hdr_view,
            tone_mapping_pipeline,
            tone_mapping_bind_group,
            hdr_sampler,
            normal_texture,
            normal_view,
            normal_bind_group,
            ssao_raw_texture,
            ssao_raw_view,
            ssao_blurred_texture,
            ssao_blurred_view,
            ssao_bind_group,
            ssao_blur_bind_group,
            ssao_pbr_sampler,
            depth_texture,
            depth_view,
            shadow_view,
            shadow_sampler,
            csm_texture,
            csm_view,
            csm_sampler,
            local_shadow_atlas,
            local_shadow_atlas_view,
            shadow_uniform,
            shadow_uniform_buffer,
            meshes: Vec::new(),
            materials: Vec::new(),
            textures,
            texture_bind_groups,
            fallback_texture_bg,
            fallback_normal_view: fallback_normal.view,
            fallback_mr_view: fallback_mr.view,
            common_sampler,
            material_bind_groups: Vec::new(),
            material_uniform_buffers: Vec::new(),
            model_buffer,
            model_bind_group,
            shadow_model_bind_group,
            handle_to_mesh_index: HashMap::new(),
            handle_to_texture_index: HashMap::new(),
            handle_to_material_index: HashMap::new(),
            cluster_manager,
            ibl_processor,
            ibl_resource,
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
            taa_velocity_tex,
            taa_velocity_view,
            taa_history_read_idx: 0,
            taa_blend_alpha,
            csm_state,
            // Virtual Geometry is initialised by default so the Nanite-like pipeline
            // is available without extra setup. It remains a no-op unless the scene
            // spawns entities with `VirtualMesh`.
            vg_system,
            current_point_lights: Vec::new(),
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
            // Force a clean history on next frame (avoid blending stale data after toggling).
            self.taa_frame_count = 0;
            self.taa_history_read_idx = 0;
            // Keep previous VP consistent to avoid huge motion vectors on the first TAA frame.
            self.taa_prev_vp_unjittered = Mat4::from_cols_array_2d(&self.camera_uniform.view_proj);
        } else if self.taa_enabled {
            // Disabling also resets so re-enabling starts clean.
            self.taa_frame_count = 0;
            self.taa_history_read_idx = 0;
        }
        self.taa_enabled = enabled;
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
        &self.surface
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
    }

    /// Reconfigure the surface and depth buffer after a window resize.
    pub fn resize(&mut self, new_width: u32, new_height: u32) {
        if new_width == 0 || new_height == 0 {
            return;
        }
        self.config.width = new_width;
        self.config.height = new_height;
        self.surface.configure(&self.device, &self.config);

        // Recompute internal resolution (TAAU input size)
        self.internal_width = ((new_width as f32) * self.render_scale).round().max(1.0) as u32;
        self.internal_height = ((new_height as f32) * self.render_scale).round().max(1.0) as u32;

        // Depth (internal)
        let (dt, dv) = create_depth_texture(&self.device, self.internal_width, self.internal_height);
        self.depth_texture = dt;
        self.depth_view = dv;

        // HDR (internal)
        let (ht, hv, hs) = create_hdr_texture(&self.device, self.internal_width, self.internal_height);
        self.hdr_texture = ht;
        self.hdr_view = hv;
        self.hdr_sampler = hs;

        // Tone Mapping Bind Group (samples from the new HDR texture; will upscale if internal < output)
        self.tone_mapping_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tone_mapping_bg_resized"),
            layout: &self.tone_mapping_pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.hdr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.hdr_sampler),
                },
            ],
        });

        // Normal + SSAO textures (internal size)
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
            self.internal_width,
            self.internal_height,
            wgpu::TextureFormat::R16Float,
            "ssao_raw",
        );
        self.ssao_raw_texture = sr;
        self.ssao_raw_view = sv;

        let (sb, sbv) = create_normal_ssao_texture(
            &self.device,
            self.internal_width,
            self.internal_height,
            wgpu::TextureFormat::R16Float,
            "ssao_blurred",
        );
        self.ssao_blurred_texture = sb;
        self.ssao_blurred_view = sbv;

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
            label: Some("ssao_bind_group"),
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
                    resource: self.ssao_pass.kernel_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.camera_buffer.as_entire_binding(),
                },
            ],
        });

        self.ssao_blur_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ssao_blur_bind_group"),
            layout: &self.ssao_pass.blur_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.ssao_raw_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.common_sampler),
                },
            ],
        });

        // Rebuild cluster buffers for the new internal dimensions so that the
        // lighting grid matches the shading resolution.
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

        log::info!("Resized to {}×{}", new_width, new_height);
    }

    // ----- Resource upload helpers -----

    /// Uploads a CPU [`Mesh`] to the GPU and returns its index (handle).
    pub fn upload_mesh(&mut self, mesh: &Mesh) -> usize {
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
        let idx = self.meshes.len();
        self.meshes.push(GpuMesh {
            vertex_buffer: vb,
            index_buffer: ib,
            index_count: mesh.index_count(),
        });
        idx
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
            .and_then(|idx| self.textures.get(idx))
            .map(|t| &t.view)
            .unwrap_or(&self.textures[0].view);

        let normal_view = material
            .normal_texture_index
            .and_then(|idx| self.textures.get(idx))
            .map(|t| &t.view)
            .unwrap_or(&self.fallback_normal_view);

        let mr_view = material
            .mr_texture_index
            .and_then(|idx| self.textures.get(idx))
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

        let idx = self.materials.len();
        self.materials.push(material);
        self.material_uniform_buffers.push(buffer);
        self.material_bind_groups.push(bind_group);
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
            .forward_pass
            .create_texture_bind_group(&self.device, &tex);
        let idx = self.textures.len();
        self.textures.push(tex);
        self.texture_bind_groups.push(bg);
        Ok(idx)
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
            .forward_pass
            .create_texture_bind_group(&self.device, &tex);
        let idx = self.textures.len();
        self.textures.push(tex);
        self.texture_bind_groups.push(bg);
        Ok(idx)
    }

    // ----- Asset-handle–based upload (for redox_asset integration) -----

    /// Returns the GPU mesh index for this handle if it has been uploaded.
    pub fn get_mesh_index(&self, handle: Handle<MeshData>) -> Option<usize> {
        self.handle_to_mesh_index.get(&handle.id()).copied()
    }

    /// Returns the GPU texture index for this handle if it has been uploaded.
    pub fn get_texture_index(&self, handle: Handle<TextureData>) -> Option<usize> {
        self.handle_to_texture_index.get(&handle.id()).copied()
    }

    /// Returns the GPU material index for this handle if it has been uploaded.
    pub fn get_material_index(&self, handle: Handle<MaterialData>) -> Option<usize> {
        self.handle_to_material_index.get(&handle.id()).copied()
    }

    /// Uploads mesh data from the asset manager and associates it with the given handle.
    /// No-op if this handle is already uploaded. Returns the mesh index.
    pub fn add_mesh_from_asset(&mut self, handle: Handle<MeshData>, mesh: &Mesh) -> Option<usize> {
        if self.handle_to_mesh_index.contains_key(&handle.id()) {
            return self.handle_to_mesh_index.get(&handle.id()).copied();
        }
        let idx = self.upload_mesh(mesh);
        self.handle_to_mesh_index.insert(handle.id(), idx);
        Some(idx)
    }

    /// Uploads image data from the asset manager and associates it with the given handle.
    /// No-op if this handle is already uploaded. Returns the texture index.
    pub fn add_texture_from_asset(
        &mut self,
        handle: Handle<TextureData>,
        img: &image::DynamicImage,
        label: &str,
    ) -> Result<Option<usize>, image::ImageError> {
        if self.handle_to_texture_index.contains_key(&handle.id()) {
            return Ok(self.handle_to_texture_index.get(&handle.id()).copied());
        }
        let tex = Texture::from_image(
            &self.device,
            &self.queue,
            img,
            label,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        )?;
        let bg = self
            .forward_pass
            .create_texture_bind_group(&self.device, &tex);
        let idx = self.textures.len();
        self.textures.push(tex);
        self.texture_bind_groups.push(bg);
        self.handle_to_texture_index.insert(handle.id(), idx);
        Ok(Some(idx))
    }

    /// Builds a GPU material from [`MaterialData`], resolving texture handles to indices.
    /// No-op if this handle is already uploaded. Returns the material index.
    pub fn add_material_from_asset(
        &mut self,
        handle: Handle<MaterialData>,
        data: &MaterialData,
    ) -> Option<usize> {
        if self.handle_to_material_index.contains_key(&handle.id()) {
            return self.handle_to_material_index.get(&handle.id()).copied();
        }
        let texture_index = data
            .albedo_handle
            .as_ref()
            .and_then(|h| self.handle_to_texture_index.get(&h.id()).copied());
        let normal_texture_index = data
            .normal_handle
            .as_ref()
            .and_then(|h| self.handle_to_texture_index.get(&h.id()).copied());
        let mr_texture_index = data
            .mr_handle
            .as_ref()
            .and_then(|h| self.handle_to_texture_index.get(&h.id()).copied());
        let material = Material {
            base_color: data.base_color,
            texture_index,
            normal_texture_index,
            mr_texture_index,
            metallic: data.metallic,
            roughness: data.roughness,
            emissive: data.emissive,
        };
        let idx = self.add_material(material);
        self.handle_to_material_index.insert(handle.id(), idx);
        Some(idx)
    }

    // ----- Rendering -----

    /// Renders a single frame given a list of [`RenderObject`]s.
    ///
    pub fn begin_pass<'a>(
        &'a self,
        encoder: &'a mut wgpu::CommandEncoder,
        view: &'a wgpu::TextureView,
    ) -> wgpu::RenderPass<'a> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("forward_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.05,
                        g: 0.05,
                        b: 0.08,
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
        })
    }

    pub fn record_draw<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        objects: &[RenderObject],
    ) {
        render_pass.set_pipeline(&self.pbr_pass.pipeline);
        render_pass.set_bind_group(0, &self.global_bind_group, &[]);
        render_pass.set_bind_group(2, &self.model_bind_group, &[]);

        for (i, obj) in objects.iter().enumerate() {
            // Material bind group (Group 2)
            let material_bg = self
                .material_bind_groups
                .get(obj.material_index)
                .unwrap_or(&self.material_bind_groups[0]);

            render_pass.set_bind_group(1, material_bg, &[]);
            if let Some(gpu_mesh) = self.meshes.get(obj.mesh_index) {
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
        self.update_global_bind_group();
    }

    /// Recreates the global bind group with current resources (includes SSAO texture + SSAO sampler + CSM).
    pub fn update_global_bind_group(&mut self) {
        self.global_bind_group = self.pbr_pass.create_global_bind_group(
            &self.device,
            &self.camera_buffer,
            &self.light_buffer,
            &self.shadow_view,
            &self.shadow_sampler,
            &self.ibl_resource.irradiance_view,
            &self.ibl_resource.prefiltered_view,
            &self.ibl_resource.brdf_lut_view,
            &self.ssao_blurred_view,
            &self.ssao_pbr_sampler,
            &self.ibl_processor.sampler,
            &self.cluster_manager.point_lights_buffer,
            &self.cluster_manager.metadata_buffer,
            &self.cluster_manager.light_indices_buffer,
            &self.cluster_manager.info_buffer,
            &self.csm_view,
            &self.csm_sampler,
            &self.local_shadow_atlas_view,
            &self.shadow_uniform_buffer,
            &self.shader_debug_buffer,
        );
    }

    /// Returns the next available swap-chain texture so callers can render
    /// 3D + UI into the same surface before calling `present()` once.
    pub fn surface_texture(&self) -> Result<wgpu::SurfaceTexture, wgpu::SurfaceError> {
        self.surface.get_current_texture()
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

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame_encoder"),
            });

        // 0. Update Buffers
        self.update_model_buffer(objects);

        // ── TAA jitter injection ────────────────────────────────────────────
        // The unjittered VP is already in `camera_uniform` (set by the caller
        // via `camera_uniform.update()` + `update_camera_buffer()`).
        // We save it, then overwrite the GPU buffer with the jittered version.
        let unjittered_vp = Mat4::from_cols_array_2d(&self.camera_uniform.view_proj);

        if self.taa_enabled {
            // Jitter is in NDC, but we want a constant ±0.5 pixel shift in the
            // *input* render resolution (TAAU).
            let jitter =
                halton_jitter_ndc(self.taa_frame_count, self.internal_width, self.internal_height);
            // Build jittered projection: start from the unjittered VP's
            // projection component. We apply the jitter directly to the
            // combined VP matrix by modifying its z-column rows 0 and 1.
            // For a combined VP matrix the same derivation holds: adding
            // delta to z_axis.x shifts NDC_x by −delta / (−1) = +delta.
            let mut jittered_vp = unjittered_vp;
            jittered_vp.z_axis.x -= jitter[0];
            jittered_vp.z_axis.y -= jitter[1];

            let mut jittered_uniform = self.camera_uniform;
            jittered_uniform.view_proj = jittered_vp.to_cols_array_2d();
            self.queue.write_buffer(
                &self.camera_buffer,
                0,
                bytemuck::bytes_of(&jittered_uniform),
            );
        }

        // 0. Update Light Uniform with Shadow Matrix
        let mut light_u = self.pbr_pass_light_uniform();

        // Compute CSM matrices using CsmState
        let light_dir = redox_math::Vec3::new(
            light_u.dir_direction[0],
            light_u.dir_direction[1],
            light_u.dir_direction[2],
        )
        .normalize();

        // Get camera matrices from unjittered view-projection matrix
        // Note: This is a simplification - we're assuming the camera is at origin looking down -Z
        // In a real implementation, we should store view and projection matrices separately
        let unjittered_vp = Mat4::from_cols_array_2d(&self.camera_uniform.view_proj);
        
        // Extract camera view and projection matrices from camera_uniform
        // Note: In a real implementation, we should store view and projection separately
        // For now, we'll use reasonable defaults
        let camera_view = look_at(
            redox_math::Vec3::new(0.0, 0.0, 5.0),  // Camera position
            redox_math::Vec3::new(0.0, 0.0, 0.0),  // Look at origin
            redox_math::Vec3::new(0.0, 1.0, 0.0),  // Up vector
        );
        
        // Use projection matrix from camera uniform if available
        // Otherwise use default
        let camera_proj = redox_math::perspective(
            std::f32::consts::FRAC_PI_4,  // 45 degrees FOV
            16.0 / 9.0,                   // Aspect ratio
            0.1,                          // Near plane
            1000.0,                       // Far plane
        );
        
        // Update CSM state and get shadow uniform with all cascade matrices
        let shadow_u = self.csm_state.update(
            camera_view,
            camera_proj,
            light_dir,
            SHADOW_SIZE,
        );
        
        // Store updated uniform
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

        // Create buffer for the first cascade (for backward compatibility with existing shadow pass)
        // Render all CSM cascades
        for cascade_idx in 0..self.csm_state.config.cascade_count.min(4) {
            // Create buffer for this cascade's matrix
            let shadow_view_proj_buffer = buffer::create_uniform_buffer(
                &self.device,
                &format!("shadow_matrix_temp_cascade{}", cascade_idx),
                bytemuck::bytes_of(&shadow_u.csm_matrices[cascade_idx]),
            );
            
            let shadow_matrix_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("shadow_matrix_bg_cascade{}", cascade_idx)),
                layout: &self.shadow_pass.bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: shadow_view_proj_buffer.as_entire_binding(),
                }],
            });

            // Create view for this cascade layer
            let csm_layer_view = self.csm_texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some(&format!("csm_layer{}_view", cascade_idx)),
                base_array_layer: cascade_idx as u32,
                array_layer_count: Some(1),
                dimension: Some(wgpu::TextureViewDimension::D2),
                ..Default::default()
            });

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
                    if let Some(gpu_mesh) = self.meshes.get(obj.mesh_index) {
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
            }
        }
        
        // --- 1b. Local Shadow Pass (for point lights) ---
        // MVP: render point-light shadows as a 6-face cube packed into a 4×2 atlas grid:
        //   row 0: PosX, NegX, PosY, NegY
        //   row 1: PosZ, NegZ, (unused), (unused)
        // This matches the WGSL sampling logic in `get_point_light_shadow`.
        let mut shadow_casting_point_light_idx: Option<usize> = None;
        let mut local_shadow_matrices = [Mat4::IDENTITY; 6];

        for (idx, light) in self.current_point_lights.iter().enumerate() {
            if !light.cast_vsm_shadows {
                continue;
            }
            shadow_casting_point_light_idx = Some(idx);

            // Compute view-proj for the 6 cube faces (RH, depth [0,1]).
            //
            // Face order must match the WGSL `cube_face_index` mapping:
            //   0 PosX, 1 NegX, 2 PosY, 3 NegY, 4 PosZ, 5 NegZ.
            let light_pos = light.position;
            let near = 0.05_f32;
            let far = light.radius.max(0.5);
            let proj = redox_math::Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, near, far);
            let face_forward_up: [(Vec3, Vec3); 6] = [
                (Vec3::X, Vec3::Y),          // PosX
                (Vec3::NEG_X, Vec3::Y),      // NegX
                (Vec3::Y, Vec3::NEG_Z),      // PosY
                (Vec3::NEG_Y, Vec3::Z),      // NegY
                (Vec3::Z, Vec3::Y),          // PosZ
                (Vec3::NEG_Z, Vec3::Y),      // NegZ
            ];
            for (fi, (fwd, up)) in face_forward_up.iter().enumerate() {
                let target = light_pos + *fwd;
                let view = redox_math::Mat4::look_at_rh(light_pos, target, *up);
                local_shadow_matrices[fi] = proj * view;
            }

            // Clear whole atlas once, then render each face into its tile using viewport+scissor.
            let atlas_w = 4096u32;
            let atlas_h = 4096u32;
            let tile_w = atlas_w / 4;
            let tile_h = atlas_h / 2;

            // Pre-create per-face shadow-matrix buffers + bind groups so they live long enough
            // for the whole render pass (wgpu requires referenced resources to outlive the pass).
            let mut face_shadow_buffers: Vec<wgpu::Buffer> = Vec::with_capacity(6);
            let mut face_shadow_bgs: Vec<wgpu::BindGroup> = Vec::with_capacity(6);
            for face_i in 0..6usize {
                let m = local_shadow_matrices[face_i].to_cols_array_2d();
                let shadow_matrix_buffer = buffer::create_uniform_buffer(
                    &self.device,
                    "local_shadow_matrix_temp_face",
                    bytemuck::bytes_of(&m),
                );
                let shadow_matrix_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("local_shadow_matrix_bg_face"),
                    layout: &self.shadow_pass.bind_group_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: shadow_matrix_buffer.as_entire_binding(),
                    }],
                });
                face_shadow_buffers.push(shadow_matrix_buffer);
                face_shadow_bgs.push(shadow_matrix_bg);
            }

            {
                let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("local_shadow_pass_point_cube"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.local_shadow_atlas_view,
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
                shadow_pass.set_bind_group(1, &self.shadow_model_bind_group, &[]);

                // Per-face draw (6 times) with per-face shadow matrix bind group.
                for face_i in 0..6usize {
                    let (col, row) = match face_i {
                        0 => (0u32, 0u32), // PosX
                        1 => (1u32, 0u32), // NegX
                        2 => (2u32, 0u32), // PosY
                        3 => (3u32, 0u32), // NegY
                        4 => (0u32, 1u32), // PosZ
                        _ => (1u32, 1u32), // NegZ
                    };
                    let x = col * tile_w;
                    let y = row * tile_h;

                    shadow_pass.set_bind_group(0, &face_shadow_bgs[face_i], &[]);

                    shadow_pass.set_viewport(
                        x as f32,
                        y as f32,
                        tile_w as f32,
                        tile_h as f32,
                        0.0,
                        1.0,
                    );
                    shadow_pass.set_scissor_rect(x, y, tile_w, tile_h);

                    for (i, obj) in objects.iter().enumerate() {
                        if let Some(gpu_mesh) = self.meshes.get(obj.mesh_index) {
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
                }
            }

            // MVP: only one shadow-casting point light for now.
            break;
        }

        // If we have a shadow caster, we need to update its PointLightGpu entry in the storage buffer
        if let Some(idx) = shadow_casting_point_light_idx {
             use crate::light::PointLightGpu;
             let gpu_lights: Vec<PointLightGpu> = self.current_point_lights
                .iter()
                .map(|l| {
                    PointLightGpu::from_point_light(l)
                })
                .collect();
             
             if idx < gpu_lights.len() {
                 let mut updated_gpu_lights = gpu_lights;
                 updated_gpu_lights[idx].shadow_matrices =
                     std::array::from_fn(|fi| local_shadow_matrices[fi].to_cols_array_2d());
                 
                 let lights_bytes = bytemuck::cast_slice::<PointLightGpu, u8>(&updated_gpu_lights);
                 self.queue.write_buffer(&self.cluster_manager.point_lights_buffer, 0, lights_bytes);
             }
        }

        // 2. Normal pass (normals + linear depth for SSAO)
        {
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
                if let Some(gpu_mesh) = self.meshes.get(obj.mesh_index) {
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

        // 3. SSAO pass (raw occlusion)
        {
            let mut ssao_render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ssao_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.ssao_raw_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 1.0,
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
            ssao_render_pass.set_pipeline(&self.ssao_pass.pipeline);
            ssao_render_pass.set_bind_group(0, &self.ssao_bind_group, &[]);
            ssao_render_pass.draw(0..3, 0..1);
        }

        // 4. SSAO blur pass
        {
            let mut blur_render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ssao_blur_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.ssao_blurred_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 1.0,
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
            blur_render_pass.set_pipeline(&self.ssao_pass.blur_pipeline);
            blur_render_pass.set_bind_group(0, &self.ssao_blur_bind_group, &[]);
            blur_render_pass.draw(0..3, 0..1);
        }

        // 5. Forward Pass (into HDR buffer)
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("forward_pass_hdr"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.hdr_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.05,
                            g: 0.05,
                            b: 0.08,
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

            let debug_lines = std::mem::take(&mut self.current_debug_lines);
            if !debug_lines.is_empty() {
                self.debug_lines_pass.upload_lines(&self.queue, &debug_lines);
            }
            self.record_draw(&mut render_pass, objects);
            // Draw virtual geometry objects
            if let Some(vg) = &self.vg_system {
                if vg.has_content() {
                    vg.render(&mut render_pass);
                }
            }
            if !debug_lines.is_empty() {
                self.debug_lines_pass.draw(&self.camera_bind_group, &mut render_pass);
            }
        }

        // ── TAA: velocity + accumulation passes ─────────────────────────────
        if self.taa_enabled {
            // --- 6a. Update velocity uniform ---
            let jittered_vp = if self.taa_frame_count == 0 {
                unjittered_vp // first frame: no jitter yet
            } else {
                let jitter = halton_jitter_ndc(
                    self.taa_frame_count,
                    self.internal_width,
                    self.internal_height,
                );
                let mut vp = unjittered_vp;
                vp.z_axis.x -= jitter[0];
                vp.z_axis.y -= jitter[1];
                vp
            };
            let inv_jittered_vp = jittered_vp.inverse();

            let vel_uniform = VelocityUniform {
                inv_curr_vp: inv_jittered_vp.to_cols_array_2d(),
                prev_vp: self.taa_prev_vp_unjittered.to_cols_array_2d(),
                screen_size: [self.internal_width as f32, self.internal_height as f32],
                _pad: [0.0; 2],
            };
            self.queue.write_buffer(
                &self.velocity_pass.uniform_buffer,
                0,
                bytemuck::bytes_of(&vel_uniform),
            );

            let velocity_bg = self.velocity_pass.create_bind_group(&self.device, &self.depth_view);

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
                vel_pass.set_bind_group(0, &velocity_bg, &[]);
                vel_pass.draw(0..3, 0..1);
            }

            // --- 6c. Update TAA uniform ---
            let is_first_frame = self.taa_frame_count == 0;
            let taa_uniform = TaaUniform {
                output_size: [self.config.width as f32, self.config.height as f32],
                input_size: [self.internal_width as f32, self.internal_height as f32],
                blend_alpha: self.taa_blend_alpha,
                reset: if is_first_frame { 1 } else { 0 },
            };
            self.queue.write_buffer(
                &self.taa_pass.uniform_buffer,
                0,
                bytemuck::bytes_of(&taa_uniform),
            );

            // Ping-pong: read from history_read_idx, write to the other
            let history_read_idx = self.taa_history_read_idx;
            let history_write_idx = 1 - history_read_idx;

            let taa_bg = self.taa_pass.create_bind_group(
                &self.device,
                &self.hdr_view,
                &self.taa_velocity_view,
                &self.taa_history_view[history_read_idx],
            );

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

            // --- 6e. Tone mapping from TAA output ---
            // Build a temporary bind group pointing at the freshly written
            // history texture (history_write_idx) instead of hdr_view.
            let taa_tone_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("taa_tone_bg"),
                layout: &self.tone_mapping_pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            &self.taa_history_view[history_write_idx],
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.hdr_sampler),
                    },
                ],
            });
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
        } else {
            // ── Standard path (no TAA): tone mapping directly from HDR ──────
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

        self.queue.submit(std::iter::once(encoder.finish()));
        Ok(())
    }

    /// Convenience wrapper: acquires a surface texture, renders into it, then
    /// presents it. Use `render_frame_into` + manual `present()` when you need
    /// to add additional passes (e.g. egui) before presentation.
    pub fn render_frame(&mut self, objects: &[RenderObject]) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let surface_view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.render_frame_into(objects, &surface_view)?;
        output.present();
        Ok(())
    }

    /// Writes the current camera uniform to the GPU and updates the global bind group.
    pub fn update_camera_buffer(&mut self) {
        self.queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::bytes_of(&self.camera_uniform),
        );
        self.update_global_bind_group();
    }

    /// Returns the current light uniform.
    pub fn pbr_pass_light_uniform(&self) -> LightUniform {
        self.light_uniform
    }

    /// Writes a new light uniform to the GPU and updates the global bind group.
    pub fn update_light_buffer(&mut self, light: &LightUniform) {
        self.light_uniform = *light;
        self.queue
            .write_buffer(&self.light_buffer, 0, bytemuck::bytes_of(light));
        self.update_global_bind_group();
    }

    /// Writes the shader debug uniform (debug_viz_mode) to the GPU.
    pub fn update_shader_debug_buffer(&mut self, mode: ShaderDebugVizMode) {
        let mut u = ShaderDebugUniform::default();
        u.debug_viz_mode = mode as u32;
        self.queue
            .write_buffer(&self.shader_debug_buffer, 0, bytemuck::bytes_of(&u));
    }

    pub fn update_model_buffer(&self, objects: &[RenderObject]) {
        let model_data: Vec<ModelUniform> = objects
            .iter()
            .map(|obj| ModelUniform {
                model: obj.model_matrix.to_cols_array_2d(),
                color: obj.color,
            })
            .collect();
        self.queue
            .write_buffer(&self.model_buffer, 0, bytemuck::cast_slice(&model_data));
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

        // Phase 1: collect all VG entities with their current data (read-only)
        type Entity = redox_ecs::Entity;
        let mut updates: Vec<(Entity, [[f32; 4]; 4], Option<crate::virtual_geometry::VGInstanceHandle>, VGAssetId)> = Vec::new();
        for entity in world.all_entities() {
            let vm = match world.get_component::<VirtualMesh>(entity) {
                Some(v) => v.clone(),
                None => continue,
            };
            let mat = match world.get_component::<crate::systems::Transform>(entity) {
                Some(t) => t.matrix().to_cols_array_2d(),
                None => continue,
            };
            updates.push((entity, mat, vm.instance_handle, vm.asset_id));
        }

        // Phase 2: apply updates and spawn new instances, then write back handles
        let mut new_handles: Vec<(Entity, crate::virtual_geometry::VGInstanceHandle)> = Vec::new();
        for (entity, mat, handle_opt, asset_id) in &updates {
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
        vg.prepare_frame(&self.queue, &vp);
    }
}

/// Helper to create a render target texture for normal or SSAO pass.
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
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
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
