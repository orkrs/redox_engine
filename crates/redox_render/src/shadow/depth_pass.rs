//! VSM shadow depth pass — renders scene geometry into dirty physical pages.
//!
//! Each dirty page is a separate render target. The CPU uploads a **page-local**
//! view-projection (see [`crate::shadow::vsm_clipmap::VsmClipmap::page_view_proj`])
//! so the page's world slice fills NDC `[-1, 1]`; the vertex shader just applies
//! that matrix and writes linear depth.

use bytemuck::{Pod, Zeroable};
use wgpu;

use super::shadow_atlas::VSM_ATLAS_FORMAT;
use crate::shader::{VSM_DEPTH_WGSL, create_shader_with_vertex_decode};

/// Per-page info uploaded as a storage buffer so the vertex shader can
/// position the triangle into the right atlas tile.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct VsmPageDrawInfo {
    /// Page-local view-projection (world → NDC for this page only).
    pub view_proj: [[f32; 4]; 4],
    /// Atlas pixel offset X.
    pub atlas_offset_x: f32,
    /// Atlas pixel offset Y.
    pub atlas_offset_y: f32,
    /// Page size in pixels (e.g. 128).
    pub page_size_px: f32,
    /// Total atlas size in pixels.
    pub atlas_size_px: f32,
}

/// Holds the GPU pipeline and layouts for the VSM depth pass.
pub struct VsmDepthPass {
    /// Depth-only pipeline.
    pub pipeline: wgpu::RenderPipeline,
    /// Layout of the bind group.
    pub bind_group_layout: wgpu::BindGroupLayout,
}

impl VsmDepthPass {
    /// Creates the pass.
    pub fn new(device: &wgpu::Device) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vsm_depth_bgl"),
            entries: &[
                // 0: models storage
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // 1: page draw info uniform (dynamic offset per dirty page)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(256),
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vsm_depth_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let shader = create_shader_with_vertex_decode(device, "vsm_depth_shader", VSM_DEPTH_WGSL);

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            cache: crate::pipeline_cache::get(),
            label: Some("vsm_depth_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(crate::mesh::compress::compressed_position_layout())],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: VSM_ATLAS_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
        });

        Self {
            pipeline,
            bind_group_layout,
        }
    }
}
