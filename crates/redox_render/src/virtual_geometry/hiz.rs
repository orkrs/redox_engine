//! Hi-Z depth pyramid for GPU occlusion culling.

use bytemuck::{Pod, Zeroable};

use crate::shader::{HIZ_COPY_WGSL, HIZ_DOWNSAMPLE_WGSL, create_shader_module};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DownsampleParams {
    src_size: [u32; 2],
    _pad: [u32; 2],
}

/// R32Float depth pyramid with per-mip views.
pub struct HiZPyramid {
    #[allow(dead_code)]
    texture: wgpu::Texture,
    views: Vec<wgpu::TextureView>,
    copy_pipeline: wgpu::ComputePipeline,
    downsample_pipeline: wgpu::ComputePipeline,
    copy_bgl: wgpu::BindGroupLayout,
    downsample_bgl: wgpu::BindGroupLayout,
    params_buffer: wgpu::Buffer,
    depth_sampler: wgpu::Sampler,
    pub width: u32,
    pub height: u32,
    pub mip_count: u32,
    ready: bool,
}

impl HiZPyramid {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let w = width.max(1);
        let h = height.max(1);
        let mip_count = (w.max(h) as f32).log2().floor() as u32 + 1;

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hiz_pyramid"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: mip_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let views: Vec<_> = (0..mip_count)
            .map(|m| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some(&format!("hiz_mip_{m}")),
                    format: Some(wgpu::TextureFormat::R32Float),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_mip_level: m,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();

        let copy_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hiz_copy_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::R32Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });

        let downsample_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hiz_downsample_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::R32Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let copy_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hiz_copy_layout"),
            bind_group_layouts: &[Some(&copy_bgl)],
            immediate_size: 0,
        });
        let down_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hiz_down_layout"),
            bind_group_layouts: &[Some(&downsample_bgl)],
            immediate_size: 0,
        });

        let copy_shader = create_shader_module(device, "hiz_copy", HIZ_COPY_WGSL);
        let down_shader = create_shader_module(device, "hiz_downsample", HIZ_DOWNSAMPLE_WGSL);

        let copy_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            cache: None,
            label: Some("hiz_copy_pipeline"),
            layout: Some(&copy_layout),
            module: &copy_shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
        });
        let downsample_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                cache: None,
                label: Some("hiz_downsample_pipeline"),
                layout: Some(&down_layout),
                module: &down_shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
            });

        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hiz_downsample_params"),
            size: std::mem::size_of::<DownsampleParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let depth_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("hiz_sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        Self {
            texture,
            views,
            copy_pipeline,
            downsample_pipeline,
            copy_bgl,
            downsample_bgl,
            params_buffer,
            depth_sampler,
            width: w,
            height: h,
            mip_count,
            ready: false,
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.width == width.max(1) && self.height == height.max(1) {
            return;
        }
        *self = Self::new(device, width, height);
    }

    pub fn is_ready(&self) -> bool {
        self.ready
    }

    pub fn view(&self, mip: u32) -> &wgpu::TextureView {
        &self.views[mip.min(self.mip_count - 1) as usize]
    }

    pub fn sampler(&self) -> &wgpu::Sampler {
        &self.depth_sampler
    }

    /// Build the pyramid from the scene depth buffer (call after depth prepass).
    pub fn build_from_depth(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        depth_view: &wgpu::TextureView,
    ) {
        let copy_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hiz_copy_bg"),
            layout: &self.copy_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.views[0]),
                },
            ],
        });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("hiz_copy"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.copy_pipeline);
            pass.set_bind_group(0, &copy_bg, &[]);
            let gx = self.width.div_ceil(8);
            let gy = self.height.div_ceil(8);
            pass.dispatch_workgroups(gx, gy, 1);
        }

        let mut src_w = self.width;
        let mut src_h = self.height;
        for mip in 1..self.mip_count {
            let dst_w = (src_w / 2).max(1);
            let dst_h = (src_h / 2).max(1);
            let params = DownsampleParams {
                src_size: [src_w, src_h],
                _pad: [0, 0],
            };
            queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&params));

            let down_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("hiz_down_mip_{mip}")),
                layout: &self.downsample_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            &self.views[(mip - 1) as usize],
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&self.views[mip as usize]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.params_buffer.as_entire_binding(),
                    },
                ],
            });

            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(&format!("hiz_downsample_{mip}")),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.downsample_pipeline);
            pass.set_bind_group(0, &down_bg, &[]);
            pass.dispatch_workgroups(dst_w.div_ceil(8), dst_h.div_ceil(8), 1);

            src_w = dst_w;
            src_h = dst_h;
        }

        self.ready = true;
    }
}
