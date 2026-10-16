//! Skybox background pass.
//!
//! Draws a fullscreen triangle into the HDR buffer **before** opaque
//! geometry, sampling the environment cubemap along the per-pixel view ray.
//! The pipeline writes depth `1.0` with `LessEqual` compare, so it passes
//! against freshly cleared depth and is then overdrawn by all scene geometry.

use crate::pass::forward::DEPTH_FORMAT;
use crate::pass::shared_layouts::multisample_state;
use crate::shader::{SKYBOX_WGSL, create_shader_module};

/// Skybox background pass: env cubemap + camera uniform in one bind group.
pub struct SkyboxPass {
    pub pipeline: wgpu::RenderPipeline,
    /// Layout for group 0: (0) env cubemap view, (1) filtering sampler,
    /// (2) camera uniform matching [`crate::camera::CameraUniform`].
    pub bind_group_layout: wgpu::BindGroupLayout,
    /// Bind group over the environment view, sampler, and camera buffer.
    /// Refresh via [`Self::refresh_bind_group`] when the environment changes.
    pub bind_group: wgpu::BindGroup,
}

impl SkyboxPass {
    /// Builds the skybox pipeline, bind-group layout, and initial bind group.
    ///
    /// `camera_buffer` must hold a [`crate::camera::CameraUniform`]
    /// (`queue.write_buffer` updates stay visible through the bind group).
    /// `env_view` is a cube-dimension view of the environment map and
    /// `env_sampler` any filtering sampler.
    pub fn new(
        device: &wgpu::Device,
        camera_buffer: &wgpu::Buffer,
        env_view: &wgpu::TextureView,
        env_sampler: &wgpu::Sampler,
        hdr_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let shader = create_shader_module(device, "skybox_shader", SKYBOX_WGSL);

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("skybox_bgl"),
            entries: &[
                // 0: environment cubemap
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::Cube,
                        multisampled: false,
                    },
                    count: None,
                },
                // 1: environment sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // 2: camera uniform
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("skybox_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("skybox_pipeline"),
            layout: Some(&layout),
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
                    format: hdr_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            // Fullscreen triangle; culling disabled so winding never bites.
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            // Writes far-plane depth first so every later opaque fragment wins.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: multisample_state(sample_count),
            multiview: None,
        });

        let bind_group = Self::build_bind_group(
            device,
            &bind_group_layout,
            camera_buffer,
            env_view,
            env_sampler,
        );

        Self {
            pipeline,
            bind_group_layout,
            bind_group,
        }
    }

    /// Recreates the group-0 bind group for a (possibly new) environment view.
    ///
    /// Call whenever the environment changes, e.g. after
    /// [`crate::RenderContext::set_environment`].
    pub fn refresh_bind_group(
        &mut self,
        device: &wgpu::Device,
        camera_buffer: &wgpu::Buffer,
        env_view: &wgpu::TextureView,
        env_sampler: &wgpu::Sampler,
    ) {
        self.bind_group = Self::build_bind_group(
            device,
            &self.bind_group_layout,
            camera_buffer,
            env_view,
            env_sampler,
        );
    }

    fn build_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        camera_buffer: &wgpu::Buffer,
        env_view: &wgpu::TextureView,
        env_sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("skybox_bind_group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(env_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(env_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: camera_buffer.as_entire_binding(),
                },
            ],
        })
    }

    /// Records one fullscreen-triangle draw. Call right after the depth clear,
    /// before opaque draws, inside the forward HDR render pass.
    pub fn draw<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}
