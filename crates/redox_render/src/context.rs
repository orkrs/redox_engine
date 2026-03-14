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
use crate::light::LightUniform;
use crate::material::Material;
use crate::mesh::Mesh;
use crate::pass::forward::{ForwardPass, ModelUniform, create_depth_texture};
use crate::pass::normal::NormalPass;
use crate::pass::pbr::PbrPass;
use crate::pass::shadow::{SHADOW_FORMAT, SHADOW_SIZE, ShadowPass};
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

        // --- Render passes ---
        let forward_pass = ForwardPass::new(&device, hdr_format);
        let pbr_pass = PbrPass::new(&device, hdr_format);
        let shadow_pass = ShadowPass::new(&device);
        let normal_pass = NormalPass::new(&device, wgpu::TextureFormat::Rgba16Float);
        let ssao_pass = SSAOPass::new(&device, &queue);

        // --- Camera uniform ---
        let camera_uniform = CameraUniform::default();
        let camera_buffer = buffer::create_uniform_buffer(
            &device,
            "camera_uniform",
            bytemuck::bytes_of(&camera_uniform),
        );

        // --- Shadow Map ---
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

        // --- Light uniform ---
        let light_uniform = LightUniform::default();
        let light_buffer = buffer::create_uniform_buffer(
            &device,
            "light_uniform",
            bytemuck::bytes_of(&light_uniform),
        );

        // --- Depth ---
        let (depth_texture, depth_view) =
            create_depth_texture(&device, config.width, config.height);

        // --- HDR Texture ---
        let hdr_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hdr_texture"),
            size: wgpu::Extent3d {
                width: config.width,
                height: config.height,
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

        // --- Normal + SSAO textures and bind groups ---
        let (normal_texture, normal_view) = create_normal_ssao_texture(
            &device,
            config.width,
            config.height,
            wgpu::TextureFormat::Rgba16Float,
            "normal",
        );
        let (ssao_raw_texture, ssao_raw_view) = create_normal_ssao_texture(
            &device,
            config.width,
            config.height,
            wgpu::TextureFormat::R16Float,
            "ssao_raw",
        );
        let (ssao_blurred_texture, ssao_blurred_view) = create_normal_ssao_texture(
            &device,
            config.width,
            config.height,
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

        // --- IBL ---
        let ibl_processor = IBLProcessor::new(&device);
        let ibl_resource = IBLResource::dummy(&device);

        // --- Clustered Forward Rendering ---
        let dummy_camera = Camera::new(std::f32::consts::PI / 4.0, 16.0 / 9.0, 0.1, 1000.0);
        let cluster_manager = ClusterManager::new(config.width, config.height, &dummy_camera, &device, &queue);

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
        );

        Self {
            device,
            queue,
            surface,
            config,
            forward_pass,
            pbr_pass,
            shadow_pass,
            normal_pass,
            ssao_pass,
            camera_buffer,
            camera_uniform,
            light_uniform,
            light_buffer,
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
        }
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

    /// Reconfigure the surface and depth buffer after a window resize.
    pub fn resize(&mut self, new_width: u32, new_height: u32) {
        if new_width == 0 || new_height == 0 {
            return;
        }
        self.config.width = new_width;
        self.config.height = new_height;
        self.surface.configure(&self.device, &self.config);

        // Depth
        let (dt, dv) = create_depth_texture(&self.device, new_width, new_height);
        self.depth_texture = dt;
        self.depth_view = dv;

        // HDR
        let (ht, hv, hs) = create_hdr_texture(&self.device, new_width, new_height);
        self.hdr_texture = ht;
        self.hdr_view = hv;
        self.hdr_sampler = hs;

        // Tone Mapping Bind Group (samples from the new HDR texture)
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

        // Normal + SSAO textures (same size as framebuffer)
        let (nt, nv) = create_normal_ssao_texture(
            &self.device,
            new_width,
            new_height,
            wgpu::TextureFormat::Rgba16Float,
            "normal",
        );
        self.normal_texture = nt;
        self.normal_view = nv;

        let (sr, sv) = create_normal_ssao_texture(
            &self.device,
            new_width,
            new_height,
            wgpu::TextureFormat::R16Float,
            "ssao_raw",
        );
        self.ssao_raw_texture = sr;
        self.ssao_raw_view = sv;

        let (sb, sbv) = create_normal_ssao_texture(
            &self.device,
            new_width,
            new_height,
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

        self.update_global_bind_group();

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

    /// Recreates the global bind group with current resources (includes SSAO texture + SSAO sampler).
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
        );
    }

    pub fn render_frame(&mut self, objects: &[RenderObject]) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let surface_view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame_encoder"),
            });

        // 0. Update Buffers
        self.update_model_buffer(objects);

        // 0. Update Light Uniform with Shadow Matrix
        let mut light_u = self.pbr_pass_light_uniform();

        let shadow_matrix = {
            let light_dir = redox_math::Vec3::new(
                light_u.dir_direction[0],
                light_u.dir_direction[1],
                light_u.dir_direction[2],
            )
            .normalize();

            let size = 25.0;
            // Use standard RH ortho instead of rh_gl which has -1..1 depth range
            let proj = redox_math::orthographic(-size, size, -size, size, -50.0, 50.0);
            let view = redox_math::look_at(
                light_dir * -20.0,
                redox_math::Vec3::ZERO,
                redox_math::Vec3::Y,
            );
            proj * view
        };

        light_u.shadow_view_proj = shadow_matrix.to_cols_array_2d();
        self.update_light_buffer(&light_u);

        // 1. Shadow Pass
        {
            let shadow_view_proj_buffer = buffer::create_uniform_buffer(
                &self.device,
                "shadow_matrix_temp",
                bytemuck::bytes_of(&shadow_matrix.to_cols_array_2d()),
            );
            let shadow_matrix_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("shadow_matrix_bg"),
                layout: &self.shadow_pass.bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: shadow_view_proj_buffer.as_entire_binding(),
                }],
            });

            let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow_pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_view,
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

            self.record_draw(&mut render_pass, objects);
        }

        // 2. Post-processing Pass (Tone Mapping)
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
            post_pass.set_bind_group(0, &self.tone_mapping_bind_group, &[]);
            post_pass.draw(0..3, 0..1); // Fullscreen triangle
        }

        self.queue.submit(std::iter::once(encoder.finish()));
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
    pub fn update_cluster_lights(&mut self, lights: &[crate::light::PointLight], camera: &Camera) {
        self.cluster_manager.update_clusters(
            lights,
            camera,
            &self.device,
            &self.queue,
        );
    }

    /// Returns cluster manager for GPU cluster buffer bindings (if needed).
    pub fn cluster_manager(&self) -> &ClusterManager {
        &self.cluster_manager
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
