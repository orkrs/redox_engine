//! GPU feedback for virtual shadow map pages: which pages do the visible pixels need?
//!
//! A compute pass marks, per visible pixel, the page of the clipmap level whose texel size
//! matches the pixel. The bitfield is copied to a small readback buffer and consumed by the
//! CPU one or two frames later ([`PageRequestPass::latest_bits`]); the clipmap then allocates
//! and renders exactly those pages.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytemuck::{Pod, Zeroable};
use redox_math::{Mat4, Vec3};

use super::vsm_clipmap::VSM_MAX_LEVELS;
use crate::shader::{VSM_REQUEST_WGSL, create_shader_module};

/// 32-bit words in the request bitfield (`level * 256 + page` bits for every level).
pub const REQUEST_WORDS: usize = VSM_MAX_LEVELS * 256 / 32;
const BITS_BYTES: u64 = (REQUEST_WORDS * 4) as u64;
const WORKGROUP: u32 = 8;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RequestParams {
    inv_view_proj: [[f32; 4]; 4],
    cam_pos: [f32; 4],
    width: u32,
    height: u32,
    pixel_angle: f32,
    _pad: f32,
}

enum Readback {
    Idle,
    Copied,
    Mapping(Arc<AtomicBool>),
}

pub struct PageRequestPass {
    pipeline: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    bits: wgpu::Buffer,
    readback: wgpu::Buffer,
    state: Readback,
    latest: Vec<u32>,
}

impl PageRequestPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = create_shader_module(device, "vsm_request", VSM_REQUEST_WGSL);
        let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty,
            count: None,
        };
        let storage = |read_only: bool| wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vsm_request_bgl"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    1,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(2, storage(true)),
                entry(3, storage(false)),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vsm_request_layout"),
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("vsm_request_pipeline"),
            layout: Some(&layout),
            module: &shader,
            entry_point: "cs_request",
            compilation_options: Default::default(),
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vsm_request_params"),
            size: std::mem::size_of::<RequestParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bits = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vsm_request_bits"),
            size: BITS_BYTES,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vsm_request_readback"),
            size: BITS_BYTES,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            bgl,
            params,
            bits,
            readback,
            state: Readback::Idle,
            latest: vec![0; REQUEST_WORDS],
        }
    }

    /// Request bits of the most recent completed readback (all zero before the first one).
    pub fn latest_bits(&self) -> &[u32] {
        &self.latest
    }

    /// Whether at least one readback has completed with any request in it.
    pub fn has_feedback(&self) -> bool {
        self.latest.iter().any(|&w| w != 0)
    }

    /// Harvests a finished readback. Call once per frame before using [`Self::latest_bits`].
    pub fn begin_frame(&mut self, device: &wgpu::Device) {
        device.poll(wgpu::Maintain::Poll);
        if let Readback::Mapping(done) = &self.state {
            if done.load(Ordering::Acquire) {
                self.harvest();
            }
        }
    }

    /// Records the marking pass and, when the readback buffer is free, the copy.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        depth_view: &wgpu::TextureView,
        vsm_info: &wgpu::Buffer,
        view_proj: Mat4,
        cam_pos: Vec3,
        proj_y_scale: f32,
        size: (u32, u32),
    ) {
        let (width, height) = size;
        let params = RequestParams {
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            cam_pos: [cam_pos.x, cam_pos.y, cam_pos.z, 1.0],
            width,
            height,
            // One pixel spans 2 / (proj_y_scale * height) world units per unit of distance.
            pixel_angle: 2.0 / (proj_y_scale.max(1.0e-4) * height.max(1) as f32),
            _pad: 0.0,
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        encoder.clear_buffer(&self.bits, 0, None);

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vsm_request_bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: vsm_info.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.bits.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("vsm_request"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            // Every second pixel in each direction.
            pass.dispatch_workgroups(
                width.div_ceil(2).div_ceil(WORKGROUP),
                height.div_ceil(2).div_ceil(WORKGROUP),
                1,
            );
        }
        if matches!(self.state, Readback::Idle) {
            encoder.copy_buffer_to_buffer(&self.bits, 0, &self.readback, 0, BITS_BYTES);
            self.state = Readback::Copied;
        }
    }

    /// Starts mapping the readback buffer. With `block` the result is waited for, so the very
    /// next frame already has feedback (used for the first few frames).
    pub fn after_submit(&mut self, device: &wgpu::Device, block: bool) {
        if !matches!(self.state, Readback::Copied) {
            return;
        }
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        self.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                if result.is_ok() {
                    flag.store(true, Ordering::Release);
                }
            });
        self.state = Readback::Mapping(done);
        if block {
            device.poll(wgpu::Maintain::Wait);
            if let Readback::Mapping(done) = &self.state {
                if done.load(Ordering::Acquire) {
                    self.harvest();
                }
            }
        }
    }

    fn harvest(&mut self) {
        {
            let view = self.readback.slice(..).get_mapped_range();
            self.latest = view
                .chunks_exact(4)
                .map(|c| u32::from_le_bytes(c.try_into().expect("4-byte chunk")))
                .collect();
        }
        self.readback.unmap();
        self.state = Readback::Idle;
    }
}
