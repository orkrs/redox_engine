//! Visibility analysis — determines which virtual pages are needed for the
//! current frame by projecting screen-space tiles through the depth buffer
//! into each clipmap level's light-space.
//!
//! Implemented as a compute pass dispatched over 8×8 screen tiles.

use wgpu;

use crate::shader::{VSM_VISIBILITY_WGSL, create_shader_module};


/// Resources for the visibility compute pass.
pub struct VisibilityPass {
    pub pipeline: wgpu::ComputePipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
}

/// Uniform uploaded each frame for the visibility shader.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VsmVisInfo {
    pub inv_vp: [[f32; 4]; 4],
    pub num_clipmap_levels: u32,
    pub screen_width: u32,
    pub screen_height: u32,
    pub _pad: u32,
}

impl VisibilityPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("vsm_visibility_bgl"),
                entries: &[
                    // 0: depth texture
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Depth,
                        },
                        count: None,
                    },
                    // 1: VsmVisInfo uniform
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // 2: clipmap levels (storage, read)
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // 3: requested pages (storage, read_write)
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vsm_visibility_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let shader = create_shader_module(device, "vsm_visibility_shader", VSM_VISIBILITY_WGSL);

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("vsm_visibility_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: "cs_visibility",
            compilation_options: Default::default(),
        });

        Self {
            pipeline,
            bind_group_layout,
        }
    }
}
