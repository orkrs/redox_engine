//! Renderer construction (device, passes, textures, bind groups).

use super::*;

impl Renderer {
    /// Initialises the entire rendering context.
    ///
    /// This is an `async` function because `wgpu` adapter and device requests
    /// are asynchronous. Wrap with `pollster::block_on` when calling from
    /// synchronous code.
    pub async fn new(window: Arc<Window>, present_mode: wgpu::PresentMode) -> Self {
        let size = window.inner_size();

        // --- Instance ---
        let mut instance_desc = wgpu::InstanceDescriptor::new_without_display_handle();
        instance_desc.backends = wgpu::Backends::all();
        let instance = wgpu::Instance::new(instance_desc);

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
                apply_limit_buckets: false,
            })
            .await
            .expect("Failed to find a suitable GPU adapter");

        log::info!("Using adapter: {:?}", adapter.get_info().name);

        // --- Device & Queue ---
        let mut required_features = wgpu::Features::empty();
        if adapter.features().contains(wgpu::Features::PRIMITIVE_INDEX) {
            required_features |= wgpu::Features::PRIMITIVE_INDEX;
        }
        // Rgba16Float MSAA×2 needs adapter-specific format features (WebGPU baseline is ×1/×4 only).
        if adapter
            .features()
            .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
        {
            required_features |= wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
        }
        required_features |= adapter.features() & crate::profiler::PROFILER_FEATURES;
        // Block-compressed (BC1-BC7) textures from KTX2 and Basis files.
        required_features |= adapter.features() & wgpu::Features::TEXTURE_COMPRESSION_BC;
        // Lets pipelines be cached on disk between runs (Vulkan).
        required_features |= adapter.features() & wgpu::Features::PIPELINE_CACHE;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("redox_device"),
                required_features,
                required_limits: wgpu::Limits::default(),
                ..Default::default()
            })
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
            color_space: Default::default(),
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
        // Before any pipeline is built, so they all pick the cache up.
        crate::pipeline_cache::install(&device, &adapter.get_info());

        let staging_ring =
            crate::resource::StagingRing::new(&device, crate::resource::DEFAULT_RING_CAPACITY);

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
        let hdr_msaa_target_count =
            pick_hdr_msaa_sample_count(adapter, hdr_format, aa_sharp || taa_enabled);
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

        // --- Classic shadow map ---
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
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
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

        let (hdr_msaa_texture, hdr_msaa_view) =
            create_hdr_msaa_texture(&device, internal_width, internal_height, msaa_sample_count);
        let (depth_msaa_texture, depth_msaa_view) =
            create_depth_msaa_texture(&device, internal_width, internal_height, msaa_sample_count);

        let hdr_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("hdr_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
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

        let (ldr_texture, ldr_view) =
            create_sharp_intermediate(&device, config.width, config.height, "sharp_ldr");
        let (smaa_out_texture, smaa_out_view) =
            create_sharp_intermediate(&device, config.width, config.height, "sharp_smaa_out");

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
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
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
        let (ssao_raw_texture, ssao_raw_view) =
            create_normal_ssao_texture(&device, half_width, half_height, gtao_fmt, "gtao_raw");
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
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let ssao_pbr_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gtao_pbr_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let vsm_shadow_pbr_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("vsm_shadow_pbr_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
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
        let object_velocity_pass = ObjectVelocityPass::new(&device);
        let prev_model_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("prev_model_storage_buffer"),
            size: (std::mem::size_of::<[[f32; 4]; 4]>() * 10000) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let object_velocity_bg = object_velocity_pass.create_bind_group(
            &device,
            &camera_buffer,
            &model_buffer,
            &prev_model_buffer,
            &velocity_pass.uniform_buffer,
        );

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
        let taa_tone_bgs: [wgpu::BindGroup; 2] = [
            make_tone_bg(&taa_history_view0),
            make_tone_bg(&taa_history_view1),
        ];

        // --- IBL ---
        let ibl_processor = IBLProcessor::new(&device);
        let ibl_resource = IBLResource::dummy(&device, &queue);

        // --- Clustered Forward Rendering ---
        let dummy_camera = Camera::new(std::f32::consts::PI / 4.0, 16.0 / 9.0, 0.1, 1000.0);
        let cluster_manager = ClusterManager::new(
            internal_width,
            internal_height,
            &dummy_camera,
            &device,
            &queue,
        );

        let scatter_light_bind_group = scatter_light_pass.create_bind_group(
            &device,
            &hdr_view,
            &depth_view,
            &hdr_sampler,
            &cluster_manager.point_lights_buffer,
        );

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
            &local_shadow_atlas_view,
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
        use crate::shadow::atlas_allocator::MAX_CASTERS;
        // One uniform + bind group per cube face per caster. Sharing 6 buffers
        // across casters races: Queue::write_buffer batches before the encoder,
        // so every face pass would sample the LAST caster's matrices.
        let face_slots = (MAX_CASTERS as usize) * 6;
        let face_matrix_buffers: Vec<wgpu::Buffer> = (0..face_slots)
            .map(|i| {
                buffer::create_uniform_buffer(&device, &format!("face_matrix_buf{i}"), &[0u8; 64])
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

        // `vsm_info_buffer` is created once, above: a second `let` here used to shadow it, so
        // the screen-space trace bind group kept an orphan buffer that was never written and
        // sun shadows stayed off until the first resize rebuilt the bind group.
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

        let gpu_profiler = crate::profiler::GpuProfiler::new(&device, &queue);
        let vsm_page_request = crate::shadow::page_request::PageRequestPass::new(&device);

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
            local_shadow_atlas,
            local_shadow_atlas_view,
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
            skin_pipeline: std::sync::OnceLock::new(),
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
            object_velocity_pass,
            prev_model_buffer,
            object_velocity_bg,
            object_motion: Default::default(),
            frame_object_ids: None,
            forward_draw_calls: std::sync::atomic::AtomicU32::new(0),
            last_cpu_timings: FrameCpuTimings::default(),
            memory_limits: crate::memory_budget::MemoryBudget::new(),
            taa_history_tex: [taa_history_tex0, taa_history_tex1],
            taa_history_view: [taa_history_view0, taa_history_view1],
            taa_blend_alpha,
            velocity_bg,
            taa_history_bgs,
            taa_tone_bgs,
            taa_velocity_tex,
            taa_velocity_view,
            taa_history_read_idx: 0,
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
            gpu_profiler,
            ibl_intensity: 1.0,
            vsm_page_request,
            vsm_request_frames: 0,
            vsm_screen_shadows: true,
            vsm_dirty_pages_last_frame: 0,
            smrt_config: crate::shadow::smrt::SmrtQuality::Quality.config(),
            vsm_shadow_frame_count: 0,
            vsm_shadow_temporal_alpha: 0.15,
            vsm_warm: false,
            prev_dynamic_centers: Vec::new(),
            vsm_pages_array_view,
            vg_bind_groups_dirty: true,
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
}
