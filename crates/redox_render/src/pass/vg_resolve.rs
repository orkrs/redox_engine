//! Visibility-buffer shading for Virtual Geometry (Phase 3).
//!
//! Two cooperating passes replace the flat-shaded debug VG path with the full
//! PBR lighting stack (IBL + clustered lights + shadows):
//!
//! 1. [`VgVisPass`] — rasterizes VG meshlets into a *visibility buffer*.
//! 2. [`VgResolvePass`] — fullscreen resolve that shades via the PBR global BG.

use crate::material::{Material, MaterialUniform};
use crate::mesh::CompressedVertex;
use crate::pass::shared_layouts::multisample_state;
use crate::shader::{
    VG_RESOLVE_WGSL, VG_VIS_WGSL, create_shader_module, create_shader_with_vertex_decode,
};
use crate::virtual_geometry::indirect::IndirectDrawBuffer;
use crate::virtual_geometry::meshlet::{MeshletDescriptor, VGInstanceData};

/// RT0: packed `(instance_index << 16) | triangle_id`.
pub const VIS_ID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
/// RT1: world-space normal.xyz + NDC depth.
pub const VIS_NORMAL_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// RT2: interpolated mesh UV.
pub const VIS_UV_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;

/// RT0 clear value meaning "no VG geometry at this pixel".
pub const VIS_ID_SENTINEL: u32 = 0xFFFF_FFFF;

/// Rasterizes VG meshlets into the visibility buffer.
pub struct VgVisPass {
    pub pipeline: wgpu::RenderPipeline,
    /// group(1): `array<VGInstanceData>` storage (binding 0, vertex stage).
    pub vis_bgl: wgpu::BindGroupLayout,
}

impl VgVisPass {
    /// Creates the visibility pass pipeline.
    pub fn new(
        device: &wgpu::Device,
        frame_bgl: &wgpu::BindGroupLayout,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        let vis_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vg_vis_instances_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vg_vis_pipeline_layout"),
            bind_group_layouts: &[frame_bgl, &vis_bgl],
            push_constant_ranges: &[],
        });

        let shader = create_shader_with_vertex_decode(device, "vg_vis_shader", VG_VIS_WGSL);

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vg_vis_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[CompressedVertex::buffer_layout()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format: VIS_ID_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: VIS_NORMAL_DEPTH_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: VIS_UV_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                ],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        Self { pipeline, vis_bgl }
    }

    pub fn create_instances_bind_group(
        &self,
        device: &wgpu::Device,
        instance_buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vg_vis_instances_bg"),
            layout: &self.vis_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: instance_buffer.as_entire_binding(),
            }],
        })
    }
}

/// Sets pipeline, bind groups, and VG vertex/index buffers on an active pass.
pub fn bind_vis_draw_state<'a>(
    pass: &mut wgpu::RenderPass<'a>,
    vis: &'a VgVisPass,
    frame_bg: &'a wgpu::BindGroup,
    instances_bg: &'a wgpu::BindGroup,
    vertex_buffer: &'a wgpu::Buffer,
    index_buffer: &'a wgpu::Buffer,
) {
    pass.set_pipeline(&vis.pipeline);
    pass.set_bind_group(0, frame_bg, &[]);
    pass.set_bind_group(1, instances_bg, &[]);
    pass.set_vertex_buffer(0, vertex_buffer.slice(..));
    pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
}

/// Fullscreen resolve shading the visibility buffer with full PBR lighting.
pub struct VgResolvePass {
    pub pipeline: wgpu::RenderPipeline,
    resolve_bgl: wgpu::BindGroupLayout,
}

impl VgResolvePass {
    pub fn new(
        device: &wgpu::Device,
        hdr_format: wgpu::TextureFormat,
        frame_bgl: &wgpu::BindGroupLayout,
        scene_bgl: &wgpu::BindGroupLayout,
        sample_count: u32,
    ) -> Self {
        let resolve_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vg_resolve_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vg_resolve_pipeline_layout"),
            bind_group_layouts: &[frame_bgl, scene_bgl, &resolve_bgl],
            push_constant_ranges: &[],
        });

        let shader = create_shader_module(device, "vg_resolve_shader", VG_RESOLVE_WGSL);

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vg_resolve_pipeline"),
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
                    format: hdr_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: multisample_state(sample_count),
            multiview: None,
        });

        Self {
            pipeline,
            resolve_bgl,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_resolve_bind_group(
        &self,
        device: &wgpu::Device,
        rt0_view: &wgpu::TextureView,
        rt1_view: &wgpu::TextureView,
        rt2_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        instances_buffer: &wgpu::Buffer,
        meshlets_buffer: &wgpu::Buffer,
        materials_buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vg_resolve_bg"),
            layout: &self.resolve_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(rt0_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(rt1_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(rt2_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: instances_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: meshlets_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: materials_buffer.as_entire_binding(),
                },
            ],
        })
    }

    pub fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        frame_bg: &'a wgpu::BindGroup,
        scene_bg: &'a wgpu::BindGroup,
        resolve_bg: &'a wgpu::BindGroup,
    ) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, frame_bg, &[]);
        pass.set_bind_group(1, scene_bg, &[]);
        pass.set_bind_group(2, resolve_bg, &[]);
        pass.draw(0..3, 0..1);
    }
}

pub fn build_material_storage(
    materials: &[Material],
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> wgpu::Buffer {
    let uniforms: Vec<MaterialUniform> = materials.iter().map(|m| m.to_uniform(device)).collect();
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("vg_material_storage"),
        size: (uniforms.len() * std::mem::size_of::<MaterialUniform>()).max(16) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    if !uniforms.is_empty() {
        queue.write_buffer(&buffer, 0, bytemuck::cast_slice(&uniforms));
    }
    buffer
}

pub fn build_meshlet_storage(
    meshlets: &[MeshletDescriptor],
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> wgpu::Buffer {
    let bytes: &[u8] = bytemuck::cast_slice(meshlets);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("vg_meshlet_storage"),
        size: bytes.len().max(16) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    if !bytes.is_empty() {
        queue.write_buffer(&buffer, 0, bytes);
    }
    buffer
}

const _: () = {
    use std::mem::size_of;
    assert!(size_of::<MeshletDescriptor>() == 64);
    assert!(size_of::<VGInstanceData>() == 112);
    assert!(size_of::<MaterialUniform>() == 112);
};

// ── Visibility-buffer render targets ──────────────────────────────────────────

/// GPU textures for the VG visibility pass (RT0–RT2).
pub struct VgVisTargets {
    pub id_texture: wgpu::Texture,
    pub id_view: wgpu::TextureView,
    pub normal_depth_texture: wgpu::Texture,
    pub normal_depth_view: wgpu::TextureView,
    pub uv_texture: wgpu::Texture,
    pub uv_view: wgpu::TextureView,
}

/// Allocates visibility-buffer attachments at internal resolution.
pub fn create_vg_vis_targets(device: &wgpu::Device, width: u32, height: u32) -> VgVisTargets {
    let make = |format: wgpu::TextureFormat, label: &str| {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        (texture, view)
    };

    let (id_texture, id_view) = make(VIS_ID_FORMAT, "vg_vis_id");
    let (normal_depth_texture, normal_depth_view) =
        make(VIS_NORMAL_DEPTH_FORMAT, "vg_vis_normal_depth");
    let (uv_texture, uv_view) = make(VIS_UV_FORMAT, "vg_vis_uv");

    VgVisTargets {
        id_texture,
        id_view,
        normal_depth_texture,
        normal_depth_view,
        uv_texture,
        uv_view,
    }
}

// ── VG shadow depth pass ──────────────────────────────────────────────────────

/// Depth-only meshlet draws for CSM / point-light shadow atlases.
pub struct VgShadowPass {
    pub pipeline: wgpu::RenderPipeline,
    pub instances_bgl: wgpu::BindGroupLayout,
}

impl VgShadowPass {
    pub fn new(device: &wgpu::Device, depth_format: wgpu::TextureFormat) -> Self {
        let shadow_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vg_shadow_matrix_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let instances_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vg_shadow_instances_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vg_shadow_pipeline_layout"),
            bind_group_layouts: &[&shadow_bgl, &instances_bgl],
            push_constant_ranges: &[],
        });

        let shader = create_shader_with_vertex_decode(
            device,
            "vg_shadow_shader",
            crate::shader::VG_SHADOW_WGSL,
        );

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vg_shadow_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[CompressedVertex::buffer_layout()],
                compilation_options: Default::default(),
            },
            fragment: None,
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        Self {
            pipeline,
            instances_bgl,
        }
    }

    pub fn create_instances_bind_group(
        &self,
        device: &wgpu::Device,
        instance_buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vg_shadow_instances_bg"),
            layout: &self.instances_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: instance_buffer.as_entire_binding(),
            }],
        })
    }

    pub fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        shadow_matrix_bg: &'a wgpu::BindGroup,
        instances_bg: &'a wgpu::BindGroup,
        vertex_buffer: &'a wgpu::Buffer,
        index_buffer: &'a wgpu::Buffer,
        indirect: &'a IndirectDrawBuffer,
    ) {
        if indirect.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, shadow_matrix_bg, &[]);
        pass.set_bind_group(1, instances_bg, &[]);
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        indirect.issue_draws(pass);
    }
}
