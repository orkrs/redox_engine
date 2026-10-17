//! Velocity (motion-vector) render pass.
//!
//! Produces an `Rgba16Float` texture containing per-pixel screen-space motion
//! vectors (xy = UV delta from previous frame to current, z = depth, w = 1).
//!
//! ## Bind group (group 0)
//! - binding 0 : `texture_depth_2d`  (current depth buffer)
//! - binding 1 : `VelocityUniform`   (inverse current jittered VP, previous VP, screen size)
//!
//! The pass runs as a fullscreen triangle (`draw(0..3, 0..1)`) directly after
//! the main forward pass (depth buffer is fully populated at that point).

use bytemuck::{Pod, Zeroable};
use redox_math::Mat4;

use crate::mesh::CompressedVertex;
use crate::pass::forward::DEPTH_FORMAT;
use crate::resource::buffer::create_uniform_buffer;
use crate::shader::{
    OBJECT_VELOCITY_WGSL, VELOCITY_WGSL, create_shader_module, create_shader_with_vertex_decode,
};

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
    /// Current Halton jitter in UV (same conversion as `TaaUniform.jitter_uv`).
    /// Written into sky pixels so TAA's jitter subtract cancels for the background.
    pub jitter_uv: [f32; 2],
}

impl Default for VelocityUniform {
    fn default() -> Self {
        let id = Mat4::IDENTITY.to_cols_array_2d();
        Self {
            inv_curr_vp: id,
            prev_vp: id,
            screen_size: [1920.0, 1080.0],
            jitter_uv: [0.0; 2],
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
    /// Creates the pass.
    pub fn new(device: &wgpu::Device) -> Self {
        // ── Bind group layout ───────────────────────────────────────────────
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
        let shader = create_shader_module(device, "velocity_shader", VELOCITY_WGSL);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("velocity_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            cache: crate::pipeline_cache::get(),
            label: Some("velocity_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
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
            multiview_mask: None,
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

/// Redraws the objects that moved since the previous frame into the velocity texture, so that
/// their pixels carry object motion and not only the camera's (see `object_velocity.wgsl`).
///
/// Runs right after [`VelocityPass`], loading its output and the forward depth buffer.
pub struct ObjectVelocityPass {
    /// Pipeline: compressed vertices, depth tested (`LessEqual`, no depth write).
    pub pipeline: wgpu::RenderPipeline,
    /// Camera, current models, previous models and the velocity uniform.
    pub bind_group_layout: wgpu::BindGroupLayout,
}

fn storage_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

impl ObjectVelocityPass {
    /// Creates the pass.
    pub fn new(device: &wgpu::Device) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("object_velocity_bgl"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX),
                storage_entry(1),
                storage_entry(2),
                uniform_entry(3, wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("object_velocity_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let shader = create_shader_with_vertex_decode(
            device,
            "object_velocity_shader",
            OBJECT_VELOCITY_WGSL,
        );
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            cache: crate::pipeline_cache::get(),
            label: Some("object_velocity_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(CompressedVertex::buffer_layout())],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
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
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
        });
        Self {
            pipeline,
            bind_group_layout,
        }
    }

    /// Binds the camera, the model buffers and the velocity uniform (they live as long as the
    /// renderer, so this is created once).
    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        camera: &wgpu::Buffer,
        models: &wgpu::Buffer,
        prev_models: &wgpu::Buffer,
        velocity_uniform: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("object_velocity_bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: models.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: prev_models.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: velocity_uniform.as_entire_binding(),
                },
            ],
        })
    }
}
