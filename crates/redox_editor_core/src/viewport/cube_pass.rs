//! Lit / wireframe / pick unit-cube pipelines (construction and geometry upload).

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct LitUniform {
    pub(super) mvp: [[f32; 4]; 4],
    pub(super) model: [[f32; 4]; 4],
    pub(super) light_dir: [f32; 3],
    pub(super) selected: f32,
    pub(super) base_color: [f32; 3],
    pub(super) mode: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct WireUniform {
    pub(super) mvp: [[f32; 4]; 4],
    pub(super) color: [f32; 4],
}

/// Lit unit-cube preview + pick (no full Renderer).
pub struct ViewportCubePass {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) vertex_buffer: wgpu::Buffer,
    pub(super) index_buffer: wgpu::Buffer,
    pub(super) uniform_buffer: wgpu::Buffer,
    pub(super) bind_group: wgpu::BindGroup,
    pub(super) index_count: u32,
    /// Real edge LineList for [`crate::ViewportShading::Wireframe`].
    pub(super) wire_pipeline: wgpu::RenderPipeline,
    pub(super) wire_vertex_buffer: wgpu::Buffer,
    pub(super) wire_index_buffer: wgpu::Buffer,
    pub(super) wire_uniform_buffer: wgpu::Buffer,
    pub(super) wire_bind_group: wgpu::BindGroup,
    pub(super) wire_index_count: u32,
    pub(super) pick_pipeline: wgpu::RenderPipeline,
    pub(super) pick_uniform_buffer: wgpu::Buffer,
    pub(super) pick_bind_group: wgpu::BindGroup,
}

impl ViewportCubePass {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader_src = r#"
struct Uniforms {
    mvp: mat4x4<f32>,
    model: mat4x4<f32>,
    light_dir: vec3<f32>,
    pub(super) selected: f32,
    base_color: vec3<f32>,
    pub(super) mode: u32,
}
@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
}
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_n: vec3<f32>,
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip = u.mvp * vec4<f32>(in.pos, 1.0);
    out.world_n = normalize((u.model * vec4<f32>(in.normal, 0.0)).xyz);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.world_n);
    var rgb: vec3<f32>;
    if (u.mode == 1u) {
        // Unlit
        rgb = u.base_color;
    } else if (u.mode == 2u) {
        // Normals
        rgb = n * 0.5 + vec3<f32>(0.5);
    } else {
        // Lit (wireframe uses a separate LineList pipeline)
        let l = normalize(-u.light_dir);
        let ndotl = max(dot(n, l), 0.0);
        let ambient = 0.18;
        rgb = u.base_color * (ambient + ndotl * 0.85);
        let h = normalize(l + vec3<f32>(0.0, 0.35, 0.9));
        let spec = pow(max(dot(n, h), 0.0), 32.0) * 0.25;
        rgb = rgb + vec3<f32>(spec);
    }
    if (u.selected > 0.5) {
        rgb = mix(rgb, vec3<f32>(1.0, 0.55, 0.12), 0.35);
        rgb = rgb + vec3<f32>(0.08, 0.04, 0.0);
    }
    return vec4<f32>(rgb, 1.0);
}
"#;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("editor_lit_cube"),
            source: wgpu::ShaderSource::Wgsl(shader_src.into()),
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lit_uniform"),
            size: std::mem::size_of::<LitUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lit_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lit_bg"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lit_pl"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            cache: None,
            label: Some("editor_lit_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 24,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 12,
                            shader_location: 1,
                        },
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
        });

        // Pick (positions only — reuse same VB with stride 24, only loc0)
        let pick_shader_src = r#"
struct Uniforms { mvp: mat4x4<f32>, id: u32, _pad0: u32, _pad1: u32, _pad2: u32 }
@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@vertex fn vs_pick(@location(0) pos: vec3<f32>, @location(1) _n: vec3<f32>) -> @builtin(position) vec4<f32> {
    return uniforms.mvp * vec4<f32>(pos, 1.0);
}
@fragment fn fs_pick() -> @location(0) u32 { return uniforms.id; }
"#;
        let pick_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("editor_pick_shader"),
            source: wgpu::ShaderSource::Wgsl(pick_shader_src.into()),
        });
        let pick_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pick_uniform"),
            size: 80,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pick_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pick_bg"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: pick_uniform_buffer.as_entire_binding(),
            }],
        });
        let pick_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            cache: None,
            label: Some("editor_pick_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &pick_shader,
                entry_point: Some("vs_pick"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 24,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 12,
                            shader_location: 1,
                        },
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &pick_shader,
                entry_point: Some("fs_pick"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::R32Uint,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
        });

        // 24 face verts: pos + normal
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cube_vb"),
            size: 24 * 24,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cube_ib"),
            size: 36 * 2,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Wireframe: 8 corners + 12 edges (LineList), real geometry — not fresnel.
        let wire_shader_src = r#"
struct Uniforms { mvp: mat4x4<f32>, color: vec4<f32> }
@group(0) @binding(0) var<uniform> u: Uniforms;
@vertex fn vs_main(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {
    return u.mvp * vec4<f32>(pos, 1.0);
}
@fragment fn fs_main() -> @location(0) vec4<f32> { return u.color; }
"#;
        let wire_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("editor_wire_cube"),
            source: wgpu::ShaderSource::Wgsl(wire_shader_src.into()),
        });
        let wire_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wire_uniform"),
            size: std::mem::size_of::<WireUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let wire_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("wire_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let wire_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("wire_bg"),
            layout: &wire_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wire_uniform_buffer.as_entire_binding(),
            }],
        });
        let wire_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("wire_pl"),
            bind_group_layouts: &[Some(&wire_bgl)],
            immediate_size: 0,
        });
        let wire_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            cache: None,
            label: Some("editor_wire_pipeline"),
            layout: Some(&wire_pl),
            vertex: wgpu::VertexState {
                module: &wire_shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    }],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &wire_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
        });
        let wire_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wire_cube_vb"),
            size: 8 * 12,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let wire_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wire_cube_ib"),
            size: 24 * 2,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            vertex_buffer,
            index_buffer,
            uniform_buffer,
            bind_group,
            index_count: 36,
            wire_pipeline,
            wire_vertex_buffer,
            wire_index_buffer,
            wire_uniform_buffer,
            wire_bind_group,
            wire_index_count: 24,
            pick_pipeline,
            pick_uniform_buffer,
            pick_bind_group,
        }
    }

    pub fn upload_geometry(&self, queue: &wgpu::Queue) {
        // Face-expanded cube with normals
        let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
            (
                [0.0, 0.0, -1.0],
                [
                    [-0.5, -0.5, -0.5],
                    [0.5, -0.5, -0.5],
                    [0.5, 0.5, -0.5],
                    [-0.5, 0.5, -0.5],
                ],
            ),
            (
                [0.0, 0.0, 1.0],
                [
                    [0.5, -0.5, 0.5],
                    [-0.5, -0.5, 0.5],
                    [-0.5, 0.5, 0.5],
                    [0.5, 0.5, 0.5],
                ],
            ),
            (
                [-1.0, 0.0, 0.0],
                [
                    [-0.5, -0.5, 0.5],
                    [-0.5, -0.5, -0.5],
                    [-0.5, 0.5, -0.5],
                    [-0.5, 0.5, 0.5],
                ],
            ),
            (
                [1.0, 0.0, 0.0],
                [
                    [0.5, -0.5, -0.5],
                    [0.5, -0.5, 0.5],
                    [0.5, 0.5, 0.5],
                    [0.5, 0.5, -0.5],
                ],
            ),
            (
                [0.0, -1.0, 0.0],
                [
                    [-0.5, -0.5, 0.5],
                    [0.5, -0.5, 0.5],
                    [0.5, -0.5, -0.5],
                    [-0.5, -0.5, -0.5],
                ],
            ),
            (
                [0.0, 1.0, 0.0],
                [
                    [-0.5, 0.5, -0.5],
                    [0.5, 0.5, -0.5],
                    [0.5, 0.5, 0.5],
                    [-0.5, 0.5, 0.5],
                ],
            ),
        ];
        let mut verts = Vec::with_capacity(24 * 6);
        let mut indices = Vec::with_capacity(36);
        for (fi, (n, corners)) in faces.iter().enumerate() {
            let base = (fi * 4) as u16;
            for c in corners {
                verts.extend_from_slice(c);
                verts.extend_from_slice(n);
            }
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&verts));
        queue.write_buffer(&self.index_buffer, 0, bytemuck::cast_slice(&indices));

        // Unit-cube corners + 12 edges for wireframe LineList.
        let h = 0.5_f32;
        let corners: [[f32; 3]; 8] = [
            [-h, -h, -h],
            [h, -h, -h],
            [h, -h, h],
            [-h, -h, h],
            [-h, h, -h],
            [h, h, -h],
            [h, h, h],
            [-h, h, h],
        ];
        let edge_idx: [u16; 24] = [
            0, 1, 1, 2, 2, 3, 3, 0, // bottom
            4, 5, 5, 6, 6, 7, 7, 4, // top
            0, 4, 1, 5, 2, 6, 3, 7, // vertical
        ];
        let mut wire_verts = Vec::with_capacity(24);
        for c in &corners {
            wire_verts.extend_from_slice(c);
        }
        queue.write_buffer(
            &self.wire_vertex_buffer,
            0,
            bytemuck::cast_slice(&wire_verts),
        );
        queue.write_buffer(&self.wire_index_buffer, 0, bytemuck::cast_slice(&edge_idx));
    }
}
