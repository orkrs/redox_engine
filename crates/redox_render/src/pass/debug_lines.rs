//! Simple debug line list pass (e.g. for occlusion rays).
//!
//! Draws line segments with per-vertex color. Used by the audio debug visualization
//! to show listener→emitter rays (green = clear, red = occluded).

use bytemuck::{Pod, Zeroable};
use redox_math::Vec3;

use crate::pass::shared_layouts::multisample_state;

/// One vertex of a debug line: position and RGBA color.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct DebugLineVertex {
    /// Position in world space.
    pub position: [f32; 3],
    /// Color (rgba).
    pub color: [f32; 4],
}

impl DebugLineVertex {
    /// Creates a vertex at `pos` with the color `(r, g, b, a)`.
    pub fn new(pos: Vec3, r: f32, g: f32, b: f32, a: f32) -> Self {
        Self {
            position: [pos.x, pos.y, pos.z],
            color: [r, g, b, a],
        }
    }
}

// Shader source lives in `shaders/debug_lines.wgsl`, embedded at compile time.

/// Pipeline and buffer for drawing debug lines (e.g. occlusion rays).
pub struct DebugLinesPass {
    /// Line pipeline.
    pub pipeline: wgpu::RenderPipeline,
    /// Staging buffer for line vertices (updated each frame).
    pub vertex_buffer: wgpu::Buffer,
    /// Current vertex count (2 per line).
    pub vertex_count: u32,
}

impl DebugLinesPass {
    /// Maximum number of lines drawn per frame.
    pub const MAX_LINES: usize = 512;
    const VERTEX_SIZE: u64 = std::mem::size_of::<DebugLineVertex>() as u64;
    const BUFFER_SIZE: u64 = Self::VERTEX_SIZE * (Self::MAX_LINES * 2) as u64;

    /// Creates the pass for the given color and depth formats.
    pub fn new(
        device: &wgpu::Device,
        camera_bgl: &wgpu::BindGroupLayout,
        sample_count: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("debug_lines_shader"),
            source: wgpu::ShaderSource::Wgsl(crate::shader::DEBUG_LINES_WGSL.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("debug_lines_layout"),
            bind_group_layouts: &[Some(camera_bgl)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            cache: crate::pipeline_cache::get(),
            label: Some("debug_lines_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: Self::VERTEX_SIZE,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x3,
                        },
                        wgpu::VertexAttribute {
                            offset: 12,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                    ],
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: multisample_state(sample_count),
            multiview_mask: None,
        });

        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("debug_lines_vertex_buffer"),
            size: Self::BUFFER_SIZE,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            vertex_buffer,
            vertex_count: 0,
        }
    }

    /// Upload line segments to the vertex buffer. Call before [`draw`](Self::draw).
    pub fn upload_lines(
        &mut self,
        queue: &wgpu::Queue,
        lines: &[(redox_math::Vec3, redox_math::Vec3, bool)],
    ) {
        let n = lines.len().min(Self::MAX_LINES);
        if n == 0 {
            self.vertex_count = 0;
            return;
        }
        let mut vertices: Vec<DebugLineVertex> = Vec::with_capacity(n * 2);
        for (start, end, occluded) in lines.iter().take(n) {
            // false = cyan (expected shadow) · true = yellow (toward sun)
            let (r, g, b) = if *occluded {
                (1.0, 0.92, 0.15)
            } else {
                (0.15, 0.95, 1.0)
            };
            vertices.push(DebugLineVertex::new(*start, r, g, b, 1.0));
            vertices.push(DebugLineVertex::new(*end, r, g, b, 1.0));
        }
        self.vertex_count = vertices.len() as u32;
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
    }

    /// Records the debug line draw into the current render pass. Call after [`upload_lines`](Self::upload_lines).
    pub fn draw<'a>(
        &'a self,
        camera_bind_group: &'a wgpu::BindGroup,
        pass: &mut wgpu::RenderPass<'a>,
    ) {
        if self.vertex_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..self.vertex_count, 0..1);
    }
}
