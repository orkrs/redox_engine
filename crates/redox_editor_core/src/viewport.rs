use crate::camera::EditorCamera;
use crate::picker::EntityPicker;
use crate::viewport_mode::QUAD_ORTHO_HALF;
use redox_math::{Transform, Vec3};
use wgpu::{Texture, TextureView};

/// Offscreen viewport for editor (Rgba8Unorm color + Depth32Float depth, sampled by egui)
pub struct EditorViewport {
    pub color_tex: Texture,
    pub color_view: TextureView,
    pub depth_tex: Texture,
    pub depth_view: TextureView,
    pub size: (u32, u32),
}

impl EditorViewport {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let w = width.max(1);
        let h = height.max(1);
        let color_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("editor_viewport_color"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let depth_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("editor_viewport_depth"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let color_view = color_tex.create_view(&Default::default());
        let depth_view = depth_tex.create_view(&Default::default());
        Self {
            color_tex,
            color_view,
            depth_tex,
            depth_view,
            size: (w, h),
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        *self = Self::new(device, width, height);
    }

    pub fn color_view(&self) -> &TextureView {
        &self.color_view
    }
    pub fn depth_view(&self) -> &TextureView {
        &self.depth_view
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LitUniform {
    mvp: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
    light_dir: [f32; 3],
    selected: f32,
    base_color: [f32; 3],
    mode: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct WireUniform {
    mvp: [[f32; 4]; 4],
    color: [f32; 4],
}

/// Lit unit-cube preview + pick (no full Renderer).
pub struct ViewportCubePass {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    index_count: u32,
    /// Real edge LineList for [`crate::ViewportShading::Wireframe`].
    wire_pipeline: wgpu::RenderPipeline,
    wire_vertex_buffer: wgpu::Buffer,
    wire_index_buffer: wgpu::Buffer,
    wire_uniform_buffer: wgpu::Buffer,
    wire_bind_group: wgpu::BindGroup,
    wire_index_count: u32,
    pick_pipeline: wgpu::RenderPipeline,
    pick_uniform_buffer: wgpu::Buffer,
    pick_bind_group: wgpu::BindGroup,
}

impl ViewportCubePass {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader_src = r#"
struct Uniforms {
    mvp: mat4x4<f32>,
    model: mat4x4<f32>,
    light_dir: vec3<f32>,
    selected: f32,
    base_color: vec3<f32>,
    mode: u32,
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
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("editor_lit_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
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
                entry_point: "fs_main",
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
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
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
            label: Some("editor_pick_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &pick_shader,
                entry_point: "vs_pick",
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
                entry_point: "fs_pick",
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
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
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
            bind_group_layouts: &[&wire_bgl],
            push_constant_ranges: &[],
        });
        let wire_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("editor_wire_pipeline"),
            layout: Some(&wire_pl),
            vertex: wgpu::VertexState {
                module: &wire_shader,
                entry_point: "vs_main",
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
                entry_point: "fs_main",
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
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
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
            ([0.0, 0.0, -1.0], [[-0.5, -0.5, -0.5], [0.5, -0.5, -0.5], [0.5, 0.5, -0.5], [-0.5, 0.5, -0.5]]),
            ([0.0, 0.0, 1.0], [[0.5, -0.5, 0.5], [-0.5, -0.5, 0.5], [-0.5, 0.5, 0.5], [0.5, 0.5, 0.5]]),
            ([-1.0, 0.0, 0.0], [[-0.5, -0.5, 0.5], [-0.5, -0.5, -0.5], [-0.5, 0.5, -0.5], [-0.5, 0.5, 0.5]]),
            ([1.0, 0.0, 0.0], [[0.5, -0.5, -0.5], [0.5, -0.5, 0.5], [0.5, 0.5, 0.5], [0.5, 0.5, -0.5]]),
            ([0.0, -1.0, 0.0], [[-0.5, -0.5, 0.5], [0.5, -0.5, 0.5], [0.5, -0.5, -0.5], [-0.5, -0.5, -0.5]]),
            ([0.0, 1.0, 0.0], [[-0.5, 0.5, -0.5], [0.5, 0.5, -0.5], [0.5, 0.5, 0.5], [-0.5, 0.5, 0.5]]),
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
        queue.write_buffer(&self.wire_vertex_buffer, 0, bytemuck::cast_slice(&wire_verts));
        queue.write_buffer(&self.wire_index_buffer, 0, bytemuck::cast_slice(&edge_idx));
    }

    /// `selected_id`: entity id to highlight, or None.
    /// `scissor`: optional pixel rect (x, y, w, h) within the offscreen target.
    /// `clear`: clear color/depth before drawing.
    ///
    /// Each entity is `write_buffer` + its own submit so uniforms are not clobbered
    /// (queue writes run before a submitted encoder, not interleaved with passes).
    pub fn render_entities(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport: &EditorViewport,
        camera: &EditorCamera,
        entities: &[(u32, Transform)],
        selected_id: Option<u32>,
        shading: crate::ViewportShading,
        scissor: Option<(u32, u32, u32, u32)>,
        clear: bool,
    ) {
        let view_proj = camera.proj_matrix() * camera.view_matrix();
        let light_dir = [0.35, -1.0, 0.25];
        let mode = shading.as_u32();

        if clear {
            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("editor_clear"),
                });
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("editor_clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &viewport.color_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.09,
                                g: 0.10,
                                b: 0.12,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &viewport.depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                if let Some((x, y, w, h)) = scissor {
                    pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
                    pass.set_scissor_rect(x, y, w, h);
                }
            }
            queue.submit(Some(encoder.finish()));
        }

        for (id, t) in entities {
            let model = t.matrix();
            let mvp = view_proj * model;

            if shading.is_wireframe() {
                let color = if selected_id == Some(*id) {
                    [1.0, 0.55, 0.12, 1.0]
                } else {
                    [0.85, 0.88, 0.92, 1.0]
                };
                let wu = WireUniform {
                    mvp: mvp.to_cols_array_2d(),
                    color,
                };
                queue.write_buffer(&self.wire_uniform_buffer, 0, bytemuck::bytes_of(&wu));
                let mut encoder =
                    device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("editor_wire_draw"),
                    });
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("editor_wire_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &viewport.color_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &viewport.depth_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    if let Some((x, y, w, h)) = scissor {
                        pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
                        pass.set_scissor_rect(x, y, w, h);
                    }
                    pass.set_pipeline(&self.wire_pipeline);
                    pass.set_bind_group(0, &self.wire_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.wire_vertex_buffer.slice(..));
                    pass.set_index_buffer(self.wire_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..self.wire_index_count, 0, 0..1);
                }
                queue.submit(Some(encoder.finish()));
                continue;
            }

            let u = LitUniform {
                mvp: mvp.to_cols_array_2d(),
                model: model.to_cols_array_2d(),
                light_dir,
                selected: if selected_id == Some(*id) { 1.0 } else { 0.0 },
                base_color: [0.62, 0.66, 0.72],
                mode,
            };
            queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&u));
            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("editor_lit_draw"),
                });
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("editor_lit_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &viewport.color_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &viewport.depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                if let Some((x, y, w, h)) = scissor {
                    pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
                    pass.set_scissor_rect(x, y, w, h);
                }
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..self.index_count, 0, 0..1);
            }
            queue.submit(Some(encoder.finish()));
        }
    }

    /// Quad layout: Top | Front / Right | Perspective into one texture.
    pub fn render_quad(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport: &EditorViewport,
        perspective: &EditorCamera,
        entities: &[(u32, Transform)],
        selected_id: Option<u32>,
        shading: crate::ViewportShading,
    ) {
        let (tw, th) = viewport.size;
        let hw = tw / 2;
        let hh = th / 2;
        let pivot = perspective.pivot;
        let half = QUAD_ORTHO_HALF;
        let mut top = EditorCamera::ortho_top(pivot, half);
        top.set_viewport_size(hw, hh);
        let mut front = EditorCamera::ortho_front(pivot, half);
        front.set_viewport_size(hw, hh);
        let mut right = EditorCamera::ortho_right(pivot, half);
        right.set_viewport_size(hw, hh);
        let mut persp = perspective.clone();
        persp.ortho_half = None;
        persp.set_viewport_size(hw, hh);

        {
            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("quad_clear"),
                });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("quad_clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &viewport.color_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.06,
                                g: 0.07,
                                b: 0.09,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &viewport.depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
            }
            queue.submit(Some(encoder.finish()));
        }

        let panes = [
            (&top, (0u32, 0u32, hw, hh)),
            (&front, (hw, 0, tw - hw, hh)),
            (&right, (0, hh, hw, th - hh)),
            (&persp, (hw, hh, tw - hw, th - hh)),
        ];
        for (cam, sc) in panes {
            {
                let mut encoder =
                    device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("quad_depth_clear"),
                    });
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("quad_depth_clear"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &viewport.color_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &viewport.depth_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.0),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    pass.set_viewport(sc.0 as f32, sc.1 as f32, sc.2 as f32, sc.3 as f32, 0.0, 1.0);
                    pass.set_scissor_rect(sc.0, sc.1, sc.2, sc.3);
                }
                queue.submit(Some(encoder.finish()));
            }
            self.render_entities(
                device,
                queue,
                viewport,
                cam,
                entities,
                selected_id,
                shading,
                Some(sc),
                false,
            );
        }
    }

    /// Back-compat: old signature without selection.
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport: &EditorViewport,
        camera: &EditorCamera,
        _time: f32,
    ) {
        self.render_entities(
            device,
            queue,
            viewport,
            camera,
            &[],
            None,
            crate::ViewportShading::Lit,
            None,
            true,
        );
    }

    pub fn render_pick_entities(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        picker: &EntityPicker,
        viewport: &EditorViewport,
        camera: &EditorCamera,
        entities: &[(u32, Transform)],
    ) {
        let view_proj = camera.proj_matrix() * camera.view_matrix();
        let id_view = picker.id_view.as_ref().expect("picker view");

        {
            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("editor_pick_clear"),
                });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("editor_pick_clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: id_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 4294967295.0,
                                g: 0.0,
                                b: 0.0,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &viewport.depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
            }
            queue.submit(Some(encoder.finish()));
        }

        for (entity_id, t) in entities {
            let mvp = view_proj * t.matrix();
            let mut data = [0u8; 80];
            data[0..64].copy_from_slice(bytemuck::bytes_of(&mvp.to_cols_array_2d()));
            data[64..68].copy_from_slice(&entity_id.to_ne_bytes());
            queue.write_buffer(&self.pick_uniform_buffer, 0, &data);
            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("editor_pick_draw"),
                });
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("editor_pick_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: id_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &viewport.depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_pipeline(&self.pick_pipeline);
                pass.set_bind_group(0, &self.pick_bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..self.index_count, 0, 0..1);
            }
            queue.submit(Some(encoder.finish()));
        }
    }

    pub fn render_pick(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        picker: &EntityPicker,
        viewport: &EditorViewport,
        camera: &EditorCamera,
        _time: f32,
        entity_id: u32,
    ) {
        self.render_pick_entities(
            device,
            queue,
            picker,
            viewport,
            camera,
            &[(entity_id, Transform::default())],
        );
    }

    pub fn render_ghost(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport: &EditorViewport,
        camera: &EditorCamera,
        ghost_transform: Transform,
    ) {
        self.render_entities(
            device,
            queue,
            viewport,
            camera,
            &[(u32::MAX - 1, ghost_transform)],
            None,
            crate::ViewportShading::Unlit,
            None,
            false,
        );
    }
}

/// Colored line overlay for grid + gizmo axes (Rgba8Unorm, no MSAA).
pub struct ViewportLinesPass {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    capacity: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LineVert {
    pos: [f32; 3],
    color: [f32; 4],
}

impl ViewportLinesPass {
    const MAX_VERTS: u32 = 4096;

    pub fn new(device: &wgpu::Device) -> Self {
        let shader_src = r#"
struct Uniforms { mvp: mat4x4<f32> }
@group(0) @binding(0) var<uniform> u: Uniforms;
struct VsOut { @builtin(position) clip: vec4<f32>, @location(0) color: vec4<f32> }
@vertex fn vs_main(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> VsOut {
    var o: VsOut;
    o.clip = u.mvp * vec4<f32>(pos, 1.0);
    o.color = color;
    return o;
}
@fragment fn fs_main(in: VsOut) -> @location(0) vec4<f32> { return in.color; }
"#;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("editor_lines"),
            source: wgpu::ShaderSource::Wgsl(shader_src.into()),
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lines_uniform"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lines_bgl"),
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
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lines_bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lines_pl"),
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("editor_lines_pipeline"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 28,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 12,
                            shader_location: 1,
                        },
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lines_vb"),
            size: (Self::MAX_VERTS as u64) * 28,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            vertex_buffer,
            uniform_buffer,
            bind_group,
            capacity: Self::MAX_VERTS,
        }
    }

    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        viewport: &EditorViewport,
        queue: &wgpu::Queue,
        camera: &EditorCamera,
        lines: &[(Vec3, Vec3, [f32; 4])],
    ) {
        if lines.is_empty() {
            return;
        }
        let mut verts: Vec<LineVert> = Vec::with_capacity(lines.len() * 2);
        for (a, b, c) in lines.iter().take(self.capacity as usize / 2) {
            verts.push(LineVert {
                pos: a.to_array(),
                color: *c,
            });
            verts.push(LineVert {
                pos: b.to_array(),
                color: *c,
            });
        }
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&verts));
        let mvp = camera.proj_matrix() * camera.view_matrix();
        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::bytes_of(&mvp.to_cols_array_2d()),
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("editor_lines_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &viewport.color_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &viewport.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..verts.len() as u32, 0..1);
    }
}

/// Build grid + world axes + translate gizmo at `gizmo_origin`.
pub fn build_editor_overlay_lines(
    gizmo_origin: Option<Vec3>,
    cell_size: f32,
) -> Vec<(Vec3, Vec3, [f32; 4])> {
    let mut lines = Vec::new();
    let half = 3.0 * cell_size;
    let grid_col = [0.25, 0.28, 0.32, 0.85];
    let axis_x = [0.85, 0.25, 0.25, 1.0];
    let axis_z = [0.25, 0.45, 0.95, 1.0];
    for i in -3..=3 {
        let p = i as f32 * cell_size;
        let col = if i == 0 { axis_x } else { grid_col };
        lines.push((Vec3::new(-half, 0.0, p), Vec3::new(half, 0.0, p), col));
        let col = if i == 0 { axis_z } else { grid_col };
        lines.push((Vec3::new(p, 0.0, -half), Vec3::new(p, 0.0, half), col));
    }
    lines.push((
        Vec3::ZERO,
        Vec3::Y * half * 0.35,
        [0.25, 0.9, 0.35, 1.0],
    ));

    if let Some(o) = gizmo_origin {
        let len = 1.35;
        let tip = 0.18;
        lines.push((o, o + Vec3::X * len, [1.0, 0.2, 0.2, 1.0]));
        lines.push((
            o + Vec3::X * len,
            o + Vec3::X * (len - tip) + Vec3::Y * tip * 0.5,
            [1.0, 0.2, 0.2, 1.0],
        ));
        lines.push((o, o + Vec3::Y * len, [0.25, 1.0, 0.35, 1.0]));
        lines.push((
            o + Vec3::Y * len,
            o + Vec3::Y * (len - tip) + Vec3::X * tip * 0.5,
            [0.25, 1.0, 0.35, 1.0],
        ));
        lines.push((o, o + Vec3::Z * len, [0.3, 0.55, 1.0, 1.0]));
        lines.push((
            o + Vec3::Z * len,
            o + Vec3::Z * (len - tip) + Vec3::Y * tip * 0.5,
            [0.3, 0.55, 1.0, 1.0],
        ));
    }
    lines
}

/// Wireframe unit cube (ghost / git old position).
pub fn ghost_cube_lines(center: Vec3, color: [f32; 4]) -> Vec<(Vec3, Vec3, [f32; 4])> {
    let h = 0.5;
    let c = [
        center + Vec3::new(-h, -h, -h),
        center + Vec3::new(h, -h, -h),
        center + Vec3::new(h, -h, h),
        center + Vec3::new(-h, -h, h),
        center + Vec3::new(-h, h, -h),
        center + Vec3::new(h, h, -h),
        center + Vec3::new(h, h, h),
        center + Vec3::new(-h, h, h),
    ];
    let edges = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];
    edges
        .into_iter()
        .map(|(a, b)| (c[a], c[b], color))
        .collect()
}

/// Vector from old → new for git diff.
pub fn ghost_move_line(old: Vec3, new: Vec3) -> (Vec3, Vec3, [f32; 4]) {
    (old, new, [1.0, 0.85, 0.2, 0.9])
}

pub fn egui_to_viewport_coords(
    egui_pos: egui::Pos2,
    viewport_rect: egui::Rect,
    _pixels_per_point: f32,
    tex_size: (u32, u32),
) -> Option<(u32, u32)> {
    // egui positions + rects are in *points*; the texture is stretched to fill the rect.
    // Do NOT multiply by pixels_per_point — that breaks picking on Windows DPI scaling.
    if !viewport_rect.contains(egui_pos) {
        return None;
    }
    let rel = egui_pos - viewport_rect.min;
    let w = viewport_rect.width().max(1e-3);
    let h = viewport_rect.height().max(1e-3);
    let uv_x = (rel.x / w).clamp(0.0, 1.0);
    let uv_y = (rel.y / h).clamp(0.0, 1.0);
    let x = (uv_x * tex_size.0 as f32).floor() as u32;
    let y = (uv_y * tex_size.1 as f32).floor() as u32;
    if x >= tex_size.0 || y >= tex_size.1 {
        None
    } else {
        Some((x.min(tex_size.0 - 1), y.min(tex_size.1 - 1)))
    }
}
