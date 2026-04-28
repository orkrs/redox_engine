//! Velocity (motion-vector) render pass.
//!
//! Produces an `Rgba16Float` texture containing per-pixel screen-space motion
//! vectors (xy = UV delta from previous frame to current, zw = 0/1).
//!
//! ## Bind group (group 0)
//! - binding 0 : `texture_depth_2d`  (current depth buffer)
//! - binding 1 : `VelocityUniform`   (inverse current jittered VP, previous VP, screen size)
//!
//! The pass runs as a fullscreen triangle (`draw(0..3, 0..1)`) directly after
//! the main forward pass (depth buffer is fully populated at that point).

use bytemuck::{Pod, Zeroable};
use redox_math::Mat4;

use crate::resource::buffer::create_uniform_buffer;
use crate::shader::manager::{VELOCITY_SHADER_SRC, create_shader_module};

/// GPU-side uniform for the velocity pass.
///
/// Updated every frame before the pass executes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct VelocityUniform {
    /// Inverse of the current frame's **jittered** view-projection matrix.
    /// Used to reconstruct world-space position from NDC + depth.
    pub inv_curr_vp: [[f32; 4]; 4],
    /// Previous frame's **unjittered** view-projection matrix.
    /// Used to find where each world point was on screen last frame.
    pub prev_vp: [[f32; 4]; 4],
    /// Viewport dimensions (width, height) in pixels.
    pub screen_size: [f32; 2],
    pub _pad: [f32; 2],
}

impl Default for VelocityUniform {
    fn default() -> Self {
        let id = Mat4::IDENTITY.to_cols_array_2d();
        Self {
            inv_curr_vp: id,
            prev_vp: id,
            screen_size: [1920.0, 1080.0],
            _pad: [0.0; 2],
        }
    }
}

/// The velocity buffer render pass.
pub struct VelocityPass {
    /// Render pipeline (fullscreen triangle, depth → motion vectors).
    pub pipeline: wgpu::RenderPipeline,
    /// Layout for the single bind group (depth + uniform).
    pub bind_group_layout: wgpu::BindGroupLayout,
    /// Persistent uniform buffer, updated each frame via `queue.write_buffer`.
    pub uniform_buffer: wgpu::Buffer,
}

impl VelocityPass {
    pub fn new(device: &wgpu::Device) -> Self {
        // ── Bind group layout ───────────────────────────────────────────────
        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("velocity_bgl"),
                entries: &[
                    // binding 0 : depth texture (sampled without comparison)
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // binding 1 : velocity uniform
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
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

        // ── Pipeline ────────────────────────────────────────────────────────
        let shader = create_shader_module(device, "velocity_shader", VELOCITY_SHADER_SRC);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("velocity_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("velocity_pipeline"),
            layout: Some(&pipeline_layout),
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
                    // Rgba16Float — xy = velocity UV, zw unused but safe on all backends
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: None,
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

        // ── Uniform buffer ──────────────────────────────────────────────────
        let uniform_buffer = create_uniform_buffer(
            device,
            "velocity_uniform",
            bytemuck::bytes_of(&VelocityUniform::default()),
        );

        Self {
            pipeline,
            bind_group_layout,
            uniform_buffer,
        }
    }

    /// Creates a bind group for this pass.
    ///
    /// Call once per frame (after updating `uniform_buffer`) because
    /// `depth_view` changes on resize.
    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        depth_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("velocity_bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
            ],
        })
    }
}
