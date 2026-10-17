use crate::resource::texture::Texture;

/// Holds the pre-processed textures for IBL.
pub struct IBLResource {
    pub irradiance_view: wgpu::TextureView,
    pub prefiltered_view: wgpu::TextureView,
    pub brdf_lut_view: wgpu::TextureView,
    pub environment_view: wgpu::TextureView, // Original cubemap
    /// Whether a real environment was loaded (`from_equirect`).
    /// `dummy()` leaves this false so consumers (e.g. the skybox pass) can
    /// skip drawing the all-black placeholder.
    env_loaded: bool,
    /// Dummy maps must stay alive: dropping them leaves the views pointing at
    /// freed GPU memory (view-dependent white/cyan flashes).
    _owned: Vec<wgpu::Texture>,
}

/// Represents a Skybox (environment map).
pub struct Skybox {
    pub cubemap: wgpu::Texture,
    pub view: wgpu::TextureView,
}

/// Processor to generate IBL maps using compute shaders.
pub struct IBLProcessor {
    pub pipeline_equirect_to_cube: wgpu::ComputePipeline,
    pub pipeline_irradiance: wgpu::ComputePipeline,
    pub pipeline_prefilter: wgpu::ComputePipeline,
    pub pipeline_brdf_lut: wgpu::ComputePipeline,
    pub sampler: wgpu::Sampler,
}

impl IBLProcessor {
    pub fn new(device: &wgpu::Device) -> Self {
        use crate::shader::{
            BRDF_LUT_WGSL, EQUIRECT_TO_CUBE_WGSL, IRRADIANCE_CONVOLUTION_WGSL,
            PREFILTER_CONVOLUTION_WGSL, create_shader_module,
        };

        // Each entry point lives in its own shader module so that naga never
        // sees two variables at the same @group/@binding with different types.
        let equirect_shader =
            create_shader_module(device, "equirect_to_cube_shader", EQUIRECT_TO_CUBE_WGSL);
        let irradiance_shader = create_shader_module(
            device,
            "irradiance_convolution_shader",
            IRRADIANCE_CONVOLUTION_WGSL,
        );
        let prefilter_shader = create_shader_module(
            device,
            "prefilter_convolution_shader",
            PREFILTER_CONVOLUTION_WGSL,
        );
        let brdf_lut_shader = create_shader_module(device, "brdf_lut_shader", BRDF_LUT_WGSL);

        // Linear filtering with clamping and trilinear mip blending.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ibl_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let pipeline_equirect_to_cube =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                cache: crate::pipeline_cache::get(),
                label: Some("equirect_to_cube_pipeline"),
                layout: None,
                module: &equirect_shader,
                entry_point: Some("equirect_to_cubemap"),
                compilation_options: Default::default(),
            });

        let pipeline_irradiance =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                cache: crate::pipeline_cache::get(),
                label: Some("irradiance_pipeline"),
                layout: None,
                module: &irradiance_shader,
                entry_point: Some("irradiance_convolution"),
                compilation_options: Default::default(),
            });

        let pipeline_prefilter = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            cache: crate::pipeline_cache::get(),
            label: Some("prefilter_pipeline"),
            layout: None,
            module: &prefilter_shader,
            entry_point: Some("prefilter_convolution"),
            compilation_options: Default::default(),
        });

        let pipeline_brdf_lut = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            cache: crate::pipeline_cache::get(),
            label: Some("brdf_lut_pipeline"),
            layout: None,
            module: &brdf_lut_shader,
            entry_point: Some("brdf_lut"),
            compilation_options: Default::default(),
        });

        Self {
            pipeline_equirect_to_cube,
            pipeline_irradiance,
            pipeline_prefilter,
            pipeline_brdf_lut,
            sampler,
        }
    }

    /// Converts an equirectangular HDR texture to a cubemap.
    pub fn generate_cubemap(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        equirect: &Texture,
        size: u32,
    ) -> Skybox {
        let cubemap = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ibl_cubemap"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 6,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });

        let view = cubemap.create_view(&wgpu::TextureViewDescriptor {
            label: Some("ibl_cubemap_view"),
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });

        let storage_view = cubemap.create_view(&wgpu::TextureViewDescriptor {
            label: Some("ibl_cubemap_storage_view"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("equirect_to_cube_bind_group"),
            layout: &self.pipeline_equirect_to_cube.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&equirect.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&storage_view),
                },
            ],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ibl_generator_encoder"),
        });
        {
            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("equirect_to_cube_pass"),
                timestamp_writes: None,
            });
            compute_pass.set_pipeline(&self.pipeline_equirect_to_cube);
            compute_pass.set_bind_group(0, &bind_group, &[]);
            compute_pass.dispatch_workgroups(size.div_ceil(8), size.div_ceil(8), 6);
        }
        queue.submit(Some(encoder.finish()));

        Skybox { cubemap, view }
    }

    /// Generates the BRDF LUT 2D texture.
    pub fn generate_brdf_lut(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let size = 512;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ibl_brdf_lut"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("brdf_lut_bind_group"),
            layout: &self.pipeline_brdf_lut.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ibl_brdf_lut_encoder"),
        });
        {
            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("brdf_lut_pass"),
                timestamp_writes: None,
            });
            compute_pass.set_pipeline(&self.pipeline_brdf_lut);
            compute_pass.set_bind_group(0, &bind_group, &[]);
            compute_pass.dispatch_workgroups(size / 8, size / 8, 1);
        }
        queue.submit(Some(encoder.finish()));

        (texture, view)
    }

    /// Generates the irradiance map from an environmental cubemap.
    pub fn generate_irradiance_map(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        skybox_view: &wgpu::TextureView,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let size = 32; // Irradiance maps are low resolution
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ibl_irradiance_map"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 6,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });

        let storage_view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("irradiance_bind_group"),
            layout: &self.pipeline_irradiance.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(skybox_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        // Wait, I need a separate bind group for the output if I used @group(1) in shader
        let output_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("irradiance_output_bind_group"),
            layout: &self.pipeline_irradiance.get_bind_group_layout(1),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&storage_view),
            }],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ibl_irradiance_encoder"),
        });
        {
            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("irradiance_pass"),
                timestamp_writes: None,
            });
            compute_pass.set_pipeline(&self.pipeline_irradiance);
            compute_pass.set_bind_group(0, &bind_group, &[]);
            compute_pass.set_bind_group(1, &output_bind_group, &[]);
            compute_pass.dispatch_workgroups(size / 8, size / 8, 6);
        }
        queue.submit(Some(encoder.finish()));

        let cube = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        (texture, cube)
    }

    /// Generates the pre-filtered environment map (Specular IBL).
    pub fn generate_prefiltered_map(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        skybox_view: &wgpu::TextureView,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let size = 128;
        let mip_levels = 5;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ibl_prefiltered_map"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 6,
            },
            mip_level_count: mip_levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("prefilter_bind_group"),
            layout: &self.pipeline_prefilter.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(skybox_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        for mip in 0..mip_levels {
            let mip_size = size >> mip;
            let roughness = mip as f32 / (mip_levels - 1) as f32;

            let storage_view = texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some(&format!("prefilter_mip_{mip}_storage_view")),
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                base_mip_level: mip,
                mip_level_count: Some(1),
                ..Default::default()
            });

            let output_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("prefilter_mip_{mip}_output_bind_group")),
                layout: &self.pipeline_prefilter.get_bind_group_layout(1),
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&storage_view),
                }],
            });

            let roughness_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("prefilter_roughness_buffer"),
                size: 4,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&roughness_buffer, 0, bytemuck::cast_slice(&[roughness]));

            let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("prefilter_uniform_bind_group"),
                layout: &self.pipeline_prefilter.get_bind_group_layout(2),
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: roughness_buffer.as_entire_binding(),
                }],
            });

            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ibl_prefilter_encoder"),
            });
            {
                let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some(&format!("prefilter_mip_{mip}_pass")),
                    timestamp_writes: None,
                });
                compute_pass.set_pipeline(&self.pipeline_prefilter);
                compute_pass.set_bind_group(0, &bind_group, &[]);
                compute_pass.set_bind_group(1, &output_bind_group, &[]);
                compute_pass.set_bind_group(2, &uniform_bind_group, &[]);
                compute_pass.dispatch_workgroups(mip_size.div_ceil(8), mip_size.div_ceil(8), 6);
            }
            queue.submit(Some(encoder.finish()));
        }

        let cube = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("ibl_prefiltered_view"),
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        (texture, cube)
    }
}

impl IBLResource {
    /// Generates a full IBL resource from an equirectangular HDR texture.
    pub fn from_equirect(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        processor: &IBLProcessor,
        equirect: &Texture,
    ) -> Self {
        let skybox = processor.generate_cubemap(device, queue, equirect, 1024);
        let (irradiance_tex, irradiance) =
            processor.generate_irradiance_map(device, queue, &skybox.view);
        let (prefiltered_tex, prefiltered) =
            processor.generate_prefiltered_map(device, queue, &skybox.view);
        let (brdf_tex, brdf_lut) = processor.generate_brdf_lut(device, queue);

        Self {
            irradiance_view: irradiance,
            prefiltered_view: prefiltered,
            brdf_lut_view: brdf_lut,
            environment_view: skybox.view,
            env_loaded: true,
            _owned: vec![skybox.cubemap, irradiance_tex, prefiltered_tex, brdf_tex],
        }
    }

    /// Returns true while no real environment has been loaded
    /// (i.e. [`Self::dummy`] is in place and the env cubemap is black).
    pub fn is_dummy(&self) -> bool {
        !self.env_loaded
    }

    /// Creates a dummy (all-black) IBL resource used before any environment is set.
    ///
    /// Textures stay alive and are cleared to zero. Uninitialized cubemap faces
    /// flash the whole scene white/cyan as the camera samples different directions.
    pub fn dummy(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let tex_usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        let dummy_lut = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dummy_ibl_brdf_lut"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: tex_usage,
            view_formats: &[],
        });

        let dummy_irradiance_cube = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dummy_ibl_irradiance"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 6,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: tex_usage,
            view_formats: &[],
        });

        let dummy_prefiltered_cube = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dummy_ibl_prefiltered"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 6,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: tex_usage,
            view_formats: &[],
        });

        let dummy_env_cube = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dummy_ibl_env"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 6,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: tex_usage,
            view_formats: &[],
        });

        write_black_2d(queue, &dummy_lut);
        write_black_cube(queue, &dummy_irradiance_cube);
        write_black_cube(queue, &dummy_prefiltered_cube);
        write_black_cube(queue, &dummy_env_cube);

        let cube_view_desc = wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        };

        Self {
            irradiance_view: dummy_irradiance_cube.create_view(&cube_view_desc),
            prefiltered_view: dummy_prefiltered_cube.create_view(&cube_view_desc),
            brdf_lut_view: dummy_lut.create_view(&wgpu::TextureViewDescriptor::default()),
            environment_view: dummy_env_cube.create_view(&cube_view_desc),
            env_loaded: false,
            _owned: vec![
                dummy_lut,
                dummy_irradiance_cube,
                dummy_prefiltered_cube,
                dummy_env_cube,
            ],
        }
    }
}

fn write_black_2d(queue: &wgpu::Queue, tex: &wgpu::Texture) {
    let row = vec![0u8; wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize];
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &row,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
}

fn write_black_cube(queue: &wgpu::Queue, tex: &wgpu::Texture) {
    let row = vec![0u8; wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize];
    for layer in 0..6u32 {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: layer,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &row,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
    }
}
