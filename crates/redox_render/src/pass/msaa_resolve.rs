//! Smart MSAA resolve pass (luma-weighted subpixel blend).
//!
//! Replaces hardware `resolve_target` with a fullscreen pass that loads each
//! MSAA sample via `textureLoad` and blends with luma-based weights:
//! `w = 1.0 / (1.0 + luma)`.
//!
//! ## Integration requirements
//!
//! The upstream HDR geometry pass **must** set `resolve_target: None` on the
//! MSAA colour attachment and use `store: StoreOp::Store` so the multisampled
//! texture retains all subpixel samples for this pass to read.
//!
//! ## Bind group (group 0)
//!
//! | binding | resource              | type                          |
//! |---------|-----------------------|-------------------------------|
//! | 0       | MSAA HDR colour       | `texture_multisampled_2d<f32>`|
//! | 1       | `MsaaResolveUniform`  | uniform buffer                |

use bytemuck::{Pod, Zeroable};

use crate::resource::buffer::create_uniform_buffer;
use crate::shader::{MSAA_RESOLVE_WGSL, create_shader_module};

/// Parameters uploaded to the MSAA resolve shader.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct MsaaResolveUniform {
    /// Number of MSAA samples to blend (2 or 4).
    pub sample_count: u32,
    pub _pad: [u32; 3],
}

impl MsaaResolveUniform {
    pub fn new(sample_count: u32) -> Self {
        Self {
            sample_count,
            _pad: [0; 3],
        }
    }
}

/// Fullscreen luma-weighted MSAA resolve into a single-sampled HDR target.
pub struct MsaaResolvePass {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
}

impl MsaaResolvePass {
    /// Creates the MSAA resolve pass.
    ///
    /// `hdr_format` should be `Rgba16Float` (standard RedOx HDR resolve target).
    /// Defaults to 4 MSAA samples; call [`Self::set_sample_count`] for 2× MSAA.
    pub fn new(device: &wgpu::Device, hdr_format: wgpu::TextureFormat) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("msaa_resolve_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: true,
                    },
                    count: None,
                },
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

        let shader = create_shader_module(device, "msaa_resolve_shader", MSAA_RESOLVE_WGSL);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("msaa_resolve_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            cache: crate::pipeline_cache::get(),
            label: Some("msaa_resolve_pipeline"),
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
                    format: hdr_format,
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

        let uniform = MsaaResolveUniform::new(4);
        let uniform_buffer =
            create_uniform_buffer(device, "msaa_resolve_uniform", bytemuck::bytes_of(&uniform));

        Self {
            pipeline,
            bind_group_layout,
            uniform_buffer,
        }
    }

    /// Updates the active MSAA sample count (2 or 4).
    pub fn set_sample_count(&self, queue: &wgpu::Queue, sample_count: u32) {
        let uniform = MsaaResolveUniform::new(sample_count);
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    /// Bind group for one resolve invocation.
    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        msaa_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("msaa_resolve_bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(msaa_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
            ],
        })
    }

    /// Records the fullscreen resolve into `output_view`.
    pub fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        bind_group: &wgpu::BindGroup,
        output_view: &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("msaa_resolve_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: output_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
