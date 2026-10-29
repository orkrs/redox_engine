//! Transparent (alpha-blended) geometry pass.
//!
//! Reuses the full PBR shader and bind-group layouts but renders with
//! alpha blending on the colour target and **depth write disabled** so
//! transparent surfaces never occlude each other or later draws. Objects are
//! drawn back-to-front by distance from the camera (sorted in
//! [`crate::RenderContext`], which owns the object list).

use crate::mesh::CompressedVertex;
use crate::pass::forward::DEPTH_FORMAT;
use crate::pass::pbr::PbrPass;
use crate::pass::shared_layouts::multisample_state;
use crate::shader::{PBR_WGSL, create_shader_with_vertex_decode};

/// Additive blend (MAT-06): `dst + src * alpha`, alpha itself is not added.
const ADDITIVE_BLEND: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::SrcAlpha,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
};

/// Alpha-blended variant of the PBR pipeline, plus its additive twin.
pub struct TransparentPass {
    /// Render pipeline for [`crate::BlendMode::Alpha`] materials.
    pub pipeline: wgpu::RenderPipeline,
    /// Render pipeline for [`crate::BlendMode::Additive`] materials.
    pub additive_pipeline: wgpu::RenderPipeline,
}

impl TransparentPass {
    /// Builds a pipeline with the PBR shader, bind-group layouts, vertex
    /// layout, and raster state of `pbr_pass`, but with `ALPHA_BLENDING`
    /// colour output and depth-write off (`LessEqual` test kept so hidden
    /// fragments behind opaque geometry are skipped).
    pub fn new(
        device: &wgpu::Device,
        pbr_pass: &PbrPass,
        hdr_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let shader = create_shader_with_vertex_decode(device, "pbr_transparent_shader", PBR_WGSL);

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pbr_transparent_pipeline_layout"),
            bind_group_layouts: &[
                Some(&pbr_pass.frame_bgl),
                Some(&pbr_pass.scene_bgl),
                Some(&pbr_pass.material_bgl),
                Some(&pbr_pass.model_bgl),
            ],
            immediate_size: 0,
        });

        let make_pipeline = |label: &str, blend: wgpu::BlendState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                cache: crate::pipeline_cache::get(),
                label: Some(label),
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
                        format: hdr_format,
                        blend: Some(blend),
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
                // Depth is tested against the opaque pass results but never
                // written: transparent objects must not occlude one another.
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: multisample_state(sample_count),
                multiview_mask: None,
            })
        };

        let pipeline = make_pipeline("pbr_transparent_pipeline", wgpu::BlendState::ALPHA_BLENDING);
        let additive_pipeline = make_pipeline("pbr_additive_pipeline", ADDITIVE_BLEND);

        Self {
            pipeline,
            additive_pipeline,
        }
    }
}
