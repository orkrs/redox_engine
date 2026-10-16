//! GPU compute meshlet culling (optional path; CPU fallback in [`super::culling`]).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytemuck::{Pod, Zeroable};

use super::indirect::IndirectDrawBuffer;
use super::meshlet::MeshletDescriptor;
use crate::shader::{VG_CULL_WGSL, create_shader_module};

/// Uniform uploaded each frame: frustum + camera + Hi-Z params.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct CullParamsUniform {
    pub planes: [[f32; 4]; 6],
    pub camera_pos: [f32; 4],
    pub view_proj: [[f32; 4]; 4],
    pub hiz_size: [f32; 2],
    pub hiz_mip_count: u32,
    pub hiz_enabled: u32,
    /// 0 = phase 1 (reset + cull), 1 = phase 2 (append Hi-Z rejects).
    pub cull_mode: u32,
    pub reject_capacity: u32,
    pub _pad: [u32; 2],
}

/// GPU compute cull pass writing into the VG indirect draw buffer.
pub struct VgGpuCull {
    pipeline: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
    params_buffer: wgpu::Buffer,
    cmd_count_buffer: wgpu::Buffer,
    reject_buffer: wgpu::Buffer,
    reject_count_buffer: wgpu::Buffer,
    reject_capacity: u32,
    count_staging: wgpu::Buffer,
    readback_pending: bool,
    map_requested: bool,
    map_ready: Arc<AtomicBool>,
}

impl VgGpuCull {
    pub fn new(device: &wgpu::Device) -> Self {
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vg_cull_bgl"),
            entries: &[
                uniform_entry(0),
                storage_read_entry(1),
                storage_read_entry(2),
                storage_rw_entry(3),
                storage_rw_entry(4),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
                storage_rw_entry(7),
                storage_rw_entry(8),
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vg_cull_pipeline_layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        let shader = create_shader_module(device, "vg_cull_shader", VG_CULL_WGSL);
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            cache: None,
            label: Some("vg_cull_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
        });

        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vg_cull_params"),
            size: std::mem::size_of::<CullParamsUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let cmd_count_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vg_cull_cmd_count"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let reject_capacity = super::meshlet::MAX_VISIBLE_COMMANDS as u32;
        let reject_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vg_cull_rejects"),
            size: (reject_capacity as u64) * 8,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let reject_count_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vg_cull_reject_count"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let count_staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vg_cull_count_staging"),
            size: 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            bgl,
            params_buffer,
            cmd_count_buffer,
            reject_buffer,
            reject_count_buffer,
            reject_capacity,
            count_staging,
            readback_pending: false,
            map_requested: false,
            map_ready: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Records the compute dispatch into `encoder` (does not submit).
    ///
    /// `reset_counters`: true for phase 1 (clears draw + reject lists).
    /// Phase 2 must pass `false` so recovered meshlets append to the same list.
    pub fn record_dispatch(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        params: &CullParamsUniform,
        instance_buffer: &wgpu::Buffer,
        meshlet_buffer: &wgpu::Buffer,
        indirect: &IndirectDrawBuffer,
        instance_count: u32,
        hiz_view: &wgpu::TextureView,
        hiz_sampler: &wgpu::Sampler,
        reset_counters: bool,
    ) {
        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(params));
        if reset_counters {
            queue.write_buffer(&self.cmd_count_buffer, 0, &0u32.to_le_bytes());
            queue.write_buffer(&self.reject_count_buffer, 0, &0u32.to_le_bytes());
        }

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vg_cull_bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: instance_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: meshlet_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: indirect.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.cmd_count_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(hiz_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(hiz_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: self.reject_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: self.reject_count_buffer.as_entire_binding(),
                },
            ],
        });

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(if params.cull_mode == 0 {
                "vg_cull_phase1"
            } else {
                "vg_cull_phase2"
            }),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        let groups = if params.cull_mode == 1 {
            self.reject_capacity.div_ceil(64)
        } else {
            instance_count.div_ceil(64)
        };
        if groups > 0 {
            pass.dispatch_workgroups(groups, 1, 1);
        }
    }

    pub fn reject_capacity(&self) -> u32 {
        self.reject_capacity
    }

    /// Consume last frame's GPU draw count. Maps staging, waits, then unmaps
    /// immediately so later `Queue::submit` calls (egui, etc.) stay valid.
    pub fn poll_readback(&mut self, device: &wgpu::Device, indirect: &mut IndirectDrawBuffer) {
        if self.map_requested {
            self.finish_mapped_read(device, indirect);
            return;
        }
        if !self.readback_pending {
            return;
        }
        self.map_requested = true;
        self.map_ready.store(false, Ordering::Release);
        let ready = Arc::clone(&self.map_ready);
        self.count_staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |_| {
                ready.store(true, Ordering::Release);
            });
        self.finish_mapped_read(device, indirect);
    }

    fn finish_mapped_read(&mut self, device: &wgpu::Device, indirect: &mut IndirectDrawBuffer) {
        if !self.map_ready.load(Ordering::Acquire) {
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
        }
        if !self.map_ready.load(Ordering::Acquire) {
            return;
        }
        let slice = self.count_staging.slice(..);
        let mapped = slice
            .get_mapped_range()
            .expect("buffer was mapped before reading");
        let count = u32::from_le_bytes(mapped[0..4].try_into().unwrap());
        drop(mapped);
        self.count_staging.unmap();
        indirect.count = count.min(indirect.capacity as u32);
        self.readback_pending = false;
        self.map_requested = false;
        self.map_ready.store(false, Ordering::Release);
    }

    /// Copy draw count to staging for readback at the start of the next frame.
    pub fn schedule_readback(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if self.map_requested {
            return;
        }
        encoder.copy_buffer_to_buffer(&self.cmd_count_buffer, 0, &self.count_staging, 0, 4);
        self.readback_pending = true;
    }

    /// Mapping happens in [`Self::poll_readback`] so the buffer is never left
    /// mapped across a later `Queue::submit`.
    pub fn kick_readback(&mut self) {}

    /// Submits `encoder`, reads back the atomic command count (blocking legacy path).
    pub fn read_command_count(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: wgpu::CommandEncoder,
        indirect: &mut IndirectDrawBuffer,
    ) -> u32 {
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vg_cull_count_staging"),
            size: 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut enc = encoder;
        enc.copy_buffer_to_buffer(&self.cmd_count_buffer, 0, &staging, 0, 4);
        queue.submit(std::iter::once(enc.finish()));

        let count = pollster::block_on(async {
            let slice = staging.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            let mapped = slice
                .get_mapped_range()
                .expect("buffer was mapped before reading");
            u32::from_le_bytes(mapped[0..4].try_into().unwrap())
        });

        let capped = count.min(indirect.capacity as u32);
        indirect.count = capped;
        capped
    }
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn storage_read_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn storage_rw_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

const _: () = {
    assert!(std::mem::size_of::<CullParamsUniform>() == 208);
    assert!(std::mem::size_of::<MeshletDescriptor>() == 64);
    assert!(std::mem::size_of::<super::meshlet::VGInstanceData>() == 112);
};
