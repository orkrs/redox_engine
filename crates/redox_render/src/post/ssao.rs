use crate::shader::manager::{SSAO_BLUR_SHADER_SRC, SSAO_SHADER_SRC, create_shader_module};
use bytemuck::{Pod, Zeroable};
use rand::Rng;
use redox_math::Vec3;

pub const SSAO_KERNEL_SIZE: usize = 64;

/// Kernel sample for SSAO (repr(C) for GPU upload).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SSAOKernelSample {
    x: f32,
    y: f32,
    z: f32,
    _pad: f32,
}

pub struct SSAOPass {
    pub pipeline: wgpu::RenderPipeline,
    pub blur_pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub blur_bind_group_layout: wgpu::BindGroupLayout,
    pub kernel_buffer: wgpu::Buffer,
    pub noise_texture: crate::resource::texture::Texture,
}

impl SSAOPass {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        // --- 1. Generate Sample Kernel ---
        let mut rng = rand::thread_rng();
        let mut ssao_kernel: Vec<SSAOKernelSample> = Vec::with_capacity(SSAO_KERNEL_SIZE);
        for i in 0..SSAO_KERNEL_SIZE {
            let mut sample = Vec3::new(
                rng.gen_range(-1.0..1.0),
                rng.gen_range(-1.0..1.0),
                rng.gen_range(0.0..1.0), // Hemisphere
            )
            .normalize();

            // Randomize distance
            sample *= rng.gen_range(0.0..1.0);

            // Scaled towards center
            let mut scale = (i as f32) / (SSAO_KERNEL_SIZE as f32);
            scale = lerp(0.1, 1.0, scale * scale);
            sample *= scale;

            ssao_kernel.push(SSAOKernelSample {
                x: sample.x,
                y: sample.y,
                z: sample.z,
                _pad: 0.0,
            });
        }

        let kernel_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ssao_kernel_buffer"),
            size: (std::mem::size_of::<SSAOKernelSample>() * SSAO_KERNEL_SIZE) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        queue.write_buffer(&kernel_buffer, 0, bytemuck::cast_slice(&ssao_kernel));

        // --- 2. Generate Noise Texture ---
        let mut ssao_noise = Vec::new();
        for _ in 0..16 {
            let noise = Vec3::new(rng.gen_range(-1.0..1.0), rng.gen_range(-1.0..1.0), 0.0);
            ssao_noise.push(noise.x);
            ssao_noise.push(noise.y);
            ssao_noise.push(noise.z);
            ssao_noise.push(0.0);
        }

        let noise_texture = SSAOPass::create_noise_texture(device, queue, &ssao_noise);

        // --- 3. Bind Group Layouts ---
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ssao_bgl"),
            entries: &[
                // Normal + Depth Texture
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // Noise Texture
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // Sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
                // Kernel Buffer
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Camera Buffer
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let blur_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("ssao_blur_bgl"),
                entries: &[
                    // Input SSAO Texture
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // Sampler
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        // --- 4. Pipelines ---
        let shader = create_shader_module(device, "ssao_shader", SSAO_SHADER_SRC);
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ssao_pipeline"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("ssao_pipeline_layout"),
                    bind_group_layouts: &[&bind_group_layout],
                    push_constant_ranges: &[],
                }),
            ),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::R16Float,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        let blur_shader = create_shader_module(device, "ssao_blur_shader", SSAO_BLUR_SHADER_SRC);
        let blur_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ssao_blur_pipeline"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("ssao_blur_pipeline_layout"),
                    bind_group_layouts: &[&blur_bind_group_layout],
                    push_constant_ranges: &[],
                }),
            ),
            vertex: wgpu::VertexState {
                module: &blur_shader,
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &blur_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::R16Float,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        Self {
            pipeline,
            blur_pipeline,
            bind_group_layout,
            blur_bind_group_layout,
            kernel_buffer,
            noise_texture,
        }
    }

    fn create_noise_texture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        noise_data: &[f32],
    ) -> crate::resource::texture::Texture {
        let size = wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ssao_noise_texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(noise_data),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * 16), // 4 floats * size 4
                rows_per_image: Some(4),
            },
            size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        crate::resource::texture::Texture {
            texture,
            view,
            sampler,
            width: 4,
            height: 4,
        }
    }
}

fn lerp(a: f32, b: f32, f: f32) -> f32 {
    a + f * (b - a)
}
