//! VSM shadow depth pass — renders scene geometry into dirty physical pages.
//!
//! For each dirty page we know its physical atlas offset and the
//! view-projection matrix of the clipmap level it belongs to.  The vertex
//! shader applies the VP transform, then scales and translates clip-space XY
//! so the output lands in the correct 128×128 region of the atlas texture.

use bytemuck::{Pod, Zeroable};
use wgpu;

use super::shadow_atlas::VSM_ATLAS_FORMAT;
use crate::mesh::Vertex;
use crate::shader::manager::create_shader_module;

/// Per-page info uploaded as a storage buffer so the vertex shader can
/// position the triangle into the right atlas tile.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct VsmPageDrawInfo {
    /// View-projection for this page's clipmap level.
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

/// WGSL source for the VSM depth shader.
pub const VSM_DEPTH_SHADER_SRC: &str = r#"
struct Model {
    model: mat4x4<f32>,
    color: vec4<f32>,
};

struct PageDrawInfo {
    view_proj: mat4x4<f32>,
    atlas_offset_x: f32,
    atlas_offset_y: f32,
    page_size_px: f32,
    atlas_size_px: f32,
};

@group(0) @binding(0) var<storage, read> models: array<Model>;
@group(0) @binding(1) var<uniform> page_info: PageDrawInfo;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @builtin(instance_index) instance_idx: u32,
};

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) depth: f32,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let model = models[input.instance_idx].model;
    let world_pos = model * vec4<f32>(input.position, 1.0);
    out.clip_pos = page_info.view_proj * world_pos;
    out.depth = out.clip_pos.z / out.clip_pos.w;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) f32 {
    return in.depth;
}
"#;

/// Holds the GPU pipeline and layouts for the VSM depth pass.
pub struct VsmDepthPass {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
}

impl VsmDepthPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
                    // 1: page draw info uniform
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vsm_depth_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let shader = create_shader_module(device, "vsm_depth_shader", VSM_DEPTH_SHADER_SRC);

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vsm_depth_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
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
            multiview: None,
        });

        Self {
            pipeline,
            bind_group_layout,
        }
    }
}
