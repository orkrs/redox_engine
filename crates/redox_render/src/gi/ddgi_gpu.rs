//! GPU DDGI probe fill (compute AABB rays) + sampling buffers for PBR.

use bytemuck::{Pod, Zeroable};
use redox_math::Vec3;

use super::ddgi::{DdgiProbeGrid, GridDimensions};
use crate::shader::{create_shader_module, DDGI_TRACE_WGSL};
use crate::systems::RenderObject;

/// Octahedral irradiance samples stored per probe (2×4 atlas).
pub const DDGI_OCT_SAMPLES: usize = 8;

/// Grid descriptor mirrored in `pbr.wgsl` (`@binding(22)`).
/// WGSL uniform layout rounds struct size to 48 bytes (16-byte alignment).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct DdgiGridGpu {
    pub origin: [f32; 4],
    pub spacing: f32,
    pub dims_x: u32,
    pub dims_y: u32,
    pub dims_z: u32,
    pub enabled: u32,
    pub _pad0: u32,
    pub _pad1: u32,
    pub _pad2: u32,
}

/// Single probe on GPU (`@binding(23)` storage array element).
///
/// Eight octahedral irradiance samples (RGB) with validity packed in `samples[0].w`.
/// Remaining `.w` channels are unused padding for 16-byte alignment.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct DdgiProbeGpu {
    pub samples: [[f32; 4]; DDGI_OCT_SAMPLES],
}

impl Default for DdgiProbeGpu {
    fn default() -> Self {
        Self {
            samples: [[0.0; 4]; DDGI_OCT_SAMPLES],
        }
    }
}

const MAX_TRACE_BOXES: usize = 256;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TraceParamsGpu {
    sun_dir: [f32; 4],
    sun_rad: [f32; 4],
    origin: [f32; 4],
    spacing: f32,
    dims_x: u32,
    dims_y: u32,
    dims_z: u32,
    box_count: u32,
    enabled: u32,
    _pad0: u32,
    _pad1: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SceneBoxGpu {
    bmin: [f32; 4],
    bmax: [f32; 4],
}

/// CPU grid + GPU buffers; probe irradiance is filled by compute rays vs AABBs.
pub struct DdgiGpu {
    pub grid: DdgiProbeGrid,
    pub grid_buffer: wgpu::Buffer,
    pub probe_buffer: wgpu::Buffer,
    /// When false, skips GPU fill (shader sees `enabled = 0`).
    pub enabled: bool,
    trace_pipeline: wgpu::ComputePipeline,
    trace_bgl: wgpu::BindGroupLayout,
    trace_params: wgpu::Buffer,
    box_buffer: wgpu::Buffer,
}

impl DdgiGpu {
    /// Builds a default 8×4×8 probe lattice centred on the origin.
    pub fn new_default(device: &wgpu::Device) -> Self {
        let spacing = 2.5;
        let dims = GridDimensions { x: 8, y: 4, z: 8 };
        let origin = Vec3::new(
            -((dims.x - 1) as f32) * spacing * 0.5,
            0.5,
            -((dims.z - 1) as f32) * spacing * 0.5,
        );
        Self::from_grid(device, DdgiProbeGrid::new(origin, spacing, dims))
    }

    pub fn from_grid(device: &wgpu::Device, grid: DdgiProbeGrid) -> Self {
        let probe_count = grid.probes.len().max(1);
        let grid_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ddgi_grid_uniform"),
            size: std::mem::size_of::<DdgiGridGpu>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let probe_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ddgi_probes_storage"),
            size: (probe_count * std::mem::size_of::<DdgiProbeGpu>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let trace_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ddgi_trace_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ddgi_trace_layout"),
            bind_group_layouts: &[&trace_bgl],
            push_constant_ranges: &[],
        });
        let shader = create_shader_module(device, "ddgi_trace", DDGI_TRACE_WGSL);
        let trace_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ddgi_trace_pipeline"),
            layout: Some(&layout),
            module: &shader,
            entry_point: "main",
            compilation_options: Default::default(),
        });
        let trace_params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ddgi_trace_params"),
            size: std::mem::size_of::<TraceParamsGpu>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let box_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ddgi_trace_boxes"),
            size: (MAX_TRACE_BOXES * std::mem::size_of::<SceneBoxGpu>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            grid,
            grid_buffer,
            probe_buffer,
            enabled: true,
            trace_pipeline,
            trace_bgl,
            trace_params,
            box_buffer,
        }
    }

    /// Writes the grid uniform. Probe irradiance is filled by [`Self::record_gpu_fill`].
    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        _delta_time: f32,
        _light_dir_to_sun: Vec3,
        _light_color: Vec3,
        _light_intensity: f32,
    ) {
        let grid_gpu = DdgiGridGpu {
            origin: [
                self.grid.origin.x,
                self.grid.origin.y,
                self.grid.origin.z,
                0.0,
            ],
            spacing: self.grid.spacing,
            dims_x: self.grid.dimensions.x as u32,
            dims_y: self.grid.dimensions.y as u32,
            dims_z: self.grid.dimensions.z as u32,
            enabled: u32::from(self.enabled),
            _pad0: 0,
            _pad1: 0,
            _pad2: 0,
        };
        queue.write_buffer(&self.grid_buffer, 0, bytemuck::bytes_of(&grid_gpu));
        if self.enabled {
            self.grid.update(_delta_time);
        }
    }

    /// Compute-trace 8 octahedral rays per probe against scene AABBs.
    pub fn record_gpu_fill(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        objects: &[RenderObject],
        light_dir_to_sun: Vec3,
        light_color: Vec3,
        light_intensity: f32,
    ) {
        if !self.enabled {
            return;
        }
        let mut boxes = Vec::with_capacity(objects.len().min(MAX_TRACE_BOXES));
        for obj in objects.iter().take(MAX_TRACE_BOXES) {
            boxes.push(SceneBoxGpu {
                bmin: [obj.bounds.min.x, obj.bounds.min.y, obj.bounds.min.z, 0.0],
                bmax: [obj.bounds.max.x, obj.bounds.max.y, obj.bounds.max.z, 0.0],
            });
        }
        if boxes.is_empty() {
            boxes.push(SceneBoxGpu {
                bmin: [0.0; 4],
                bmax: [0.0; 4],
            });
        }
        queue.write_buffer(&self.box_buffer, 0, bytemuck::cast_slice(&boxes));
        let sun = light_dir_to_sun.normalize_or_zero();
        let rad = light_color * light_intensity.max(0.0) * 0.12;
        let params = TraceParamsGpu {
            sun_dir: [sun.x, sun.y, sun.z, 0.0],
            sun_rad: [rad.x, rad.y, rad.z, 0.0],
            origin: [
                self.grid.origin.x,
                self.grid.origin.y,
                self.grid.origin.z,
                0.0,
            ],
            spacing: self.grid.spacing,
            dims_x: self.grid.dimensions.x as u32,
            dims_y: self.grid.dimensions.y as u32,
            dims_z: self.grid.dimensions.z as u32,
            box_count: boxes.len() as u32,
            enabled: 1,
            _pad0: 0,
            _pad1: 0,
        };
        queue.write_buffer(&self.trace_params, 0, bytemuck::bytes_of(&params));

        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ddgi_trace_bg"),
            layout: &self.trace_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.trace_params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.box_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.probe_buffer.as_entire_binding(),
                },
            ],
        });
        let count = (self.grid.dimensions.x * self.grid.dimensions.y * self.grid.dimensions.z)
            .max(1) as u32;
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ddgi_trace"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.trace_pipeline);
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups((count + 63) / 64, 1, 1);
    }
}

/// Octahedral encode (unit vector → [0,1]²).
fn oct_encode(n: Vec3) -> (f32, f32) {
    let n = n / (n.x.abs() + n.y.abs() + n.z.abs()).max(1e-6);
    let mut u = n.x;
    let mut v = n.z;
    if n.y < 0.0 {
        let ou = u;
        u = (1.0 - v.abs()) * sign_nonzero(ou);
        v = (1.0 - ou.abs()) * sign_nonzero(v);
    }
    ((u * 0.5 + 0.5).clamp(0.0, 1.0), (v * 0.5 + 0.5).clamp(0.0, 1.0))
}

fn sign_nonzero(x: f32) -> f32 {
    if x >= 0.0 {
        1.0
    } else {
        -1.0
    }
}

#[allow(dead_code)]
fn oct_texel_uv(index: usize) -> (f32, f32) {
    // 2×4 atlas covering the octahedron.
    let x = (index % 2) as f32;
    let y = (index / 2) as f32;
    ((x + 0.5) / 2.0, (y + 0.5) / 4.0)
}

#[allow(dead_code)]
fn oct_decode(uv: (f32, f32)) -> Vec3 {
    let mut f = Vec3::new(uv.0 * 2.0 - 1.0, 0.0, uv.1 * 2.0 - 1.0);
    f.y = 1.0 - f.x.abs() - f.z.abs();
    if f.y < 0.0 {
        let ox = f.x;
        f.x = (1.0 - f.z.abs()) * sign_nonzero(ox);
        f.z = (1.0 - ox.abs()) * sign_nonzero(f.z);
    }
    f.normalize_or_zero()
}

/// CPU-side octahedral sample (used by unit tests / offline tools).
#[allow(dead_code)]
pub(crate) fn sample_oct_cpu(samples: &[Vec3; DDGI_OCT_SAMPLES], dir: Vec3) -> Vec3 {
    let (u, v) = oct_encode(dir);
    let fx = u * 2.0 - 0.5;
    let fy = v * 4.0 - 0.5;
    let x0 = fx.floor().clamp(0.0, 1.0) as usize;
    let y0 = fy.floor().clamp(0.0, 3.0) as usize;
    let x1 = (x0 + 1).min(1);
    let y1 = (y0 + 1).min(3);
    let tx = fx - x0 as f32;
    let ty = fy - y0 as f32;
    let s00 = samples[y0 * 2 + x0];
    let s10 = samples[y0 * 2 + x1];
    let s01 = samples[y1 * 2 + x0];
    let s11 = samples[y1 * 2 + x1];
    let a = s00.lerp(s10, tx);
    let b = s01.lerp(s11, tx);
    a.lerp(b, ty)
}

const _: () = {
    assert!(std::mem::size_of::<DdgiGridGpu>() == 48);
    assert!(std::mem::size_of::<DdgiProbeGpu>() == 16 * DDGI_OCT_SAMPLES);
};
