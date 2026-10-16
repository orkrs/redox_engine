//! GPU particle billboards: compute compaction + indirect instanced draw.
//!
//! CPU simulation lives in `redox_vfx`; this module uploads [`GpuParticle`]
//! slices and draws camera-facing quads. An optional compute pass can re-sim
//! on GPU when [`ParticleGpuSystem::gpu_sim_enabled`] is true.

use bytemuck::{Pod, Zeroable};
use redox_vfx::{GpuParticle, ParticleSimUniforms, ParticleSystem};

use crate::camera::CameraUniform;
use crate::pass::shared_layouts::DEPTH_FORMAT;
use crate::shader::{PARTICLE_BILLBOARD_WGSL, PARTICLE_SIM_WGSL};

/// Matches WGSL `DrawIndirect` (instance_count may be atomic in sim shader).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct ParticleDrawIndirect {
    pub vertex_count: u32,
    pub instance_count: u32,
    pub first_vertex: u32,
    pub first_instance: u32,
}

/// GPU resources for particle simulation and billboard drawing.
pub struct ParticleGpuSystem {
    pub capacity: u32,
    pub gpu_sim_enabled: bool,
    /// Alive count after last upload / sim (for CPU-side queries).
    pub last_alive_count: u32,

    particle_buffer_a: wgpu::Buffer,
    particle_buffer_b: wgpu::Buffer,
    /// When true, the draw pass reads `particle_buffer_a`.
    draw_from_a: bool,
    #[allow(dead_code)]
    alive_counter: wgpu::Buffer,
    indirect_buffer: wgpu::Buffer,
    sim_uniform_buffer: wgpu::Buffer,

    reset_pipeline: wgpu::ComputePipeline,
    sim_pipeline: wgpu::ComputePipeline,
    finalize_pipeline: wgpu::ComputePipeline,
    sim_bind_group_a: wgpu::BindGroup,
    sim_bind_group_b: wgpu::BindGroup,

    draw_pipeline: wgpu::RenderPipeline,
    draw_pipeline_reactive: wgpu::RenderPipeline,
    draw_bind_group: wgpu::BindGroup,
    camera_buffer: wgpu::Buffer,

    ping: bool,
}

impl ParticleGpuSystem {
    pub fn new(device: &wgpu::Device, capacity: u32, hdr_format: wgpu::TextureFormat) -> Self {
        let capacity = capacity.max(1);
        let particle_bytes = (capacity as u64) * GpuParticle::STRIDE as u64;

        let particle_buffer_a = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle_storage_a"),
            size: particle_bytes,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let particle_buffer_b = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle_storage_b"),
            size: particle_bytes,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let alive_counter = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle_alive_counter"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let indirect_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle_indirect"),
            size: std::mem::size_of::<ParticleDrawIndirect>() as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let sim_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle_sim_uniforms"),
            size: std::mem::size_of::<ParticleSimUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle_camera"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let sim_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particle_sim_shader"),
            source: wgpu::ShaderSource::Wgsl(PARTICLE_SIM_WGSL.into()),
        });
        let draw_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particle_billboard_shader"),
            source: wgpu::ShaderSource::Wgsl(PARTICLE_BILLBOARD_WGSL.into()),
        });

        let sim_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle_sim_bgl"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::COMPUTE),
                storage_entry(1, true),
                storage_entry(2, false),
                storage_entry(3, false),
                storage_entry(4, false),
            ],
        });

        let sim_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particle_sim_layout"),
            bind_group_layouts: &[&sim_bgl],
            push_constant_ranges: &[],
        });

        let reset_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("particle_reset_pipeline"),
            layout: Some(&sim_layout),
            module: &sim_shader,
            entry_point: "cs_reset",
            compilation_options: Default::default(),
        });
        let sim_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("particle_sim_pipeline"),
            layout: Some(&sim_layout),
            module: &sim_shader,
            entry_point: "cs_main",
            compilation_options: Default::default(),
        });
        let finalize_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("particle_finalize_pipeline"),
            layout: Some(&sim_layout),
            module: &sim_shader,
            entry_point: "cs_finalize",
            compilation_options: Default::default(),
        });

        let make_sim_bg = |read: &wgpu::Buffer, write: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("particle_sim_bg"),
                layout: &sim_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: sim_uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: read.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: write.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: alive_counter.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: indirect_buffer.as_entire_binding(),
                    },
                ],
            })
        };

        let sim_bind_group_a = make_sim_bg(&particle_buffer_a, &particle_buffer_b);
        let sim_bind_group_b = make_sim_bg(&particle_buffer_b, &particle_buffer_a);

        let draw_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle_draw_bgl"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let draw_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particle_draw_layout"),
            bind_group_layouts: &[&draw_bgl],
            push_constant_ranges: &[],
        });

        let blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };

        let draw_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particle_draw_pipeline"),
            layout: Some(&draw_layout),
            vertex: wgpu::VertexState {
                module: &draw_shader,
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &draw_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: hdr_format,
                    blend: Some(blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        let draw_pipeline_reactive = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particle_draw_pipeline_reactive"),
            layout: Some(&draw_layout),
            vertex: wgpu::VertexState {
                module: &draw_shader,
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &draw_shader,
                entry_point: "fs_main",
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format: hdr_format,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::R8Unorm,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::RED,
                    }),
                ],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        let draw_bind_group = Self::make_draw_bind_group(
            device,
            &draw_bgl,
            &camera_buffer,
            &particle_buffer_a,
        );

        Self {
            capacity,
            gpu_sim_enabled: true,
            last_alive_count: 0,
            particle_buffer_a,
            particle_buffer_b,
            draw_from_a: true,
            alive_counter,
            indirect_buffer,
            sim_uniform_buffer,
            reset_pipeline,
            sim_pipeline,
            finalize_pipeline,
            sim_bind_group_a,
            sim_bind_group_b,
            draw_pipeline,
            draw_pipeline_reactive,
            draw_bind_group,
            camera_buffer,
            ping: false,
        }
    }

    /// Upload CPU-simulated particles from [`ParticleSystem`].
    pub fn upload_from_vfx(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        system: &ParticleSystem,
        camera: &CameraUniform,
    ) {
        self.upload_slice(
            device,
            queue,
            system.buffer().raw(),
            system.alive_count(),
            camera,
        );
    }

    /// Upload a raw particle slice (typically alive prefix of a VFX buffer).
    pub fn upload_slice(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        particles: &[GpuParticle],
        alive_count: usize,
        camera: &CameraUniform,
    ) {
        let alive = alive_count.min(particles.len()).min(self.capacity as usize);
        self.last_alive_count = alive as u32;

        queue.write_buffer(
            &self.particle_buffer_a,
            0,
            bytemuck::cast_slice(&particles[..alive.min(particles.len())]),
        );
        self.draw_from_a = true;
        self.refresh_draw_bind_group(device);

        let args = ParticleDrawIndirect {
            vertex_count: 4,
            instance_count: alive as u32,
            first_vertex: 0,
            first_instance: 0,
        };
        queue.write_buffer(&self.indirect_buffer, 0, bytemuck::bytes_of(&args));
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(camera));
    }

    fn draw_buffer(&self) -> &wgpu::Buffer {
        if self.draw_from_a {
            &self.particle_buffer_a
        } else {
            &self.particle_buffer_b
        }
    }

    fn refresh_draw_bind_group(&mut self, device: &wgpu::Device) {
        let draw_bgl = self.draw_pipeline.get_bind_group_layout(0);
        self.draw_bind_group = Self::make_draw_bind_group(
            device,
            &draw_bgl,
            &self.camera_buffer,
            self.draw_buffer(),
        );
    }

    fn make_draw_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        camera_buffer: &wgpu::Buffer,
        particle_buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle_draw_bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: particle_buffer.as_entire_binding(),
                },
            ],
        })
    }

    /// Optional GPU re-simulation (integrate + compact + indirect args).
    pub fn record_gpu_sim(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        uniforms: &ParticleSimUniforms,
    ) {
        if !self.gpu_sim_enabled {
            return;
        }

        queue.write_buffer(&self.sim_uniform_buffer, 0, bytemuck::bytes_of(uniforms));

        let use_b = self.ping;
        self.ping = !self.ping;
        self.draw_from_a = use_b;
        self.refresh_draw_bind_group(device);

        let sim_bg = if use_b {
            &self.sim_bind_group_b
        } else {
            &self.sim_bind_group_a
        };

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("particle_sim_reset"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.reset_pipeline);
            pass.set_bind_group(0, sim_bg, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        {
            let groups = (self.capacity + 255) / 256;
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("particle_sim_integrate"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.sim_pipeline);
            pass.set_bind_group(0, sim_bg, &[]);
            pass.dispatch_workgroups(groups, 1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("particle_sim_finalize"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.finalize_pipeline);
            pass.set_bind_group(0, sim_bg, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
    }

    /// Draw billboards after opaque geometry; optionally marks reactive mask.
    pub fn record_draw<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        reactive_view: Option<&'a wgpu::TextureView>,
    ) {
        if self.last_alive_count == 0 && !self.gpu_sim_enabled {
            return;
        }
        let pipeline = if reactive_view.is_some() {
            &self.draw_pipeline_reactive
        } else {
            &self.draw_pipeline
        };
        render_pass.set_pipeline(pipeline);
        render_pass.set_bind_group(0, &self.draw_bind_group, &[]);
        render_pass.draw_indirect(&self.indirect_buffer, 0);
    }

    /// Begin a particle render pass (HDR + optional reactive R8).
    pub fn begin_draw_pass<'a>(
        &'a self,
        encoder: &'a mut wgpu::CommandEncoder,
        hdr_view: &'a wgpu::TextureView,
        depth_view: &'a wgpu::TextureView,
        reactive_view: Option<&'a wgpu::TextureView>,
    ) -> wgpu::RenderPass<'a> {
        let color_attachments: &[Option<wgpu::RenderPassColorAttachment>] =
            if let Some(reactive) = reactive_view {
                &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: hdr_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: reactive,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ]
            } else {
                &[Some(wgpu::RenderPassColorAttachment {
                    view: hdr_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })]
            };

        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("particle_pass"),
            color_attachments,
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        })
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

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
