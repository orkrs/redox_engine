//! Alpha-tested shadow casters (MAT-06 step 3): the classic atlas pass and the VSM page pass
//! get a variant that samples the material's albedo alpha and discards below 0.5. Only
//! materials with [`crate::material::Material::alpha_test`] use it; every other caster keeps
//! the plain pipelines, so scenes without masked materials render exactly as before.

use super::shadow_atlas::VSM_ATLAS_FORMAT;
use crate::pass::shadow::SHADOW_FORMAT;
use crate::shader::{SHADOW_MASKED_WGSL, VSM_DEPTH_MASKED_WGSL, create_shader_with_vertex_decode};

/// The two masked depth pipelines.
pub struct MaskedShadow {
    /// Point-light atlas pass: groups 0 light matrix, 1 models, 2 material.
    pub atlas: wgpu::RenderPipeline,
    /// VSM page pass: groups 0 models and page info, 1 material.
    pub vsm: wgpu::RenderPipeline,
}

impl MaskedShadow {
    /// Builds both pipelines against the existing layouts of the plain passes.
    pub fn new(
        device: &wgpu::Device,
        shadow_bgl: &wgpu::BindGroupLayout,
        shadow_model_bgl: &wgpu::BindGroupLayout,
        vsm_bgl: &wgpu::BindGroupLayout,
        material_bgl: &wgpu::BindGroupLayout,
    ) -> Self {
        let atlas_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow_masked_pipeline_layout"),
            bind_group_layouts: &[Some(shadow_bgl), Some(shadow_model_bgl), Some(material_bgl)],
            immediate_size: 0,
        });
        let atlas_shader =
            create_shader_with_vertex_decode(device, "shadow_masked_shader", SHADOW_MASKED_WGSL);
        let atlas = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            cache: crate::pipeline_cache::get(),
            label: Some("shadow_masked_pipeline"),
            layout: Some(&atlas_layout),
            vertex: wgpu::VertexState {
                module: &atlas_shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(crate::mesh::compress::compressed_position_uv_layout())],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &atlas_shader,
                entry_point: Some("fs_main"),
                targets: &[],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SHADOW_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
        });

        let vsm_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vsm_depth_masked_pipeline_layout"),
            bind_group_layouts: &[Some(vsm_bgl), Some(material_bgl)],
            immediate_size: 0,
        });
        let vsm_shader = create_shader_with_vertex_decode(
            device,
            "vsm_depth_masked_shader",
            VSM_DEPTH_MASKED_WGSL,
        );
        let vsm = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            cache: crate::pipeline_cache::get(),
            label: Some("vsm_depth_masked_pipeline"),
            layout: Some(&vsm_layout),
            vertex: wgpu::VertexState {
                module: &vsm_shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(crate::mesh::compress::compressed_position_uv_layout())],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &vsm_shader,
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
                // Leaves are thin: both faces cast.
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
        });
        Self { atlas, vsm }
    }
}
