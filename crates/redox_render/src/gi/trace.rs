//! GPU probe ray trace against the global SDF (`shaders/gi_trace.wgsl`).
//!
//! Each probe casts `rays` rays; results land in `ray_data` as `(radiance, hit distance)`
//! at index `probe * rays + ray`. The bounce term from the previous frame's probes is added
//! once the probe atlas exists (GI-01 step 4); for now the trace is direct light and emission.
//! ASSUMPTION: the bounce term is deferred to step 4, so the red-wall bounce test of step 3
//! is not written here.

use bytemuck::{Pod, Zeroable};
use redox_math::Vec3;

use crate::sdf::global_gpu::GlobalSdfGpu;
use crate::shader::{GI_TRACE_WGSL, create_shader_module};

/// Largest ray count per probe; sizes the ray buffer.
pub const MAX_RAYS_PER_PROBE: u32 = 128;
/// Bytes per ray record: `vec4<f32>`.
const RAY_RECORD_BYTES: u64 = 16;

/// Trace parameters mirrored by `struct TraceParams` in `gi_trace.wgsl` (80 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GiTraceParamsGpu {
    /// Probe lattice origin (xyz) and spacing (w).
    pub grid_origin: [f32; 4],
    /// Probe counts (xyz) and rays per probe (w).
    pub grid_dims: [u32; 4],
    /// Unit direction towards the sun (xyz).
    pub sun_dir: [f32; 4],
    /// Sun radiance (xyz).
    pub sun_rad: [f32; 4],
    /// Frame index (x) and probe count (y).
    pub frame: [u32; 4],
}

const _: () = assert!(std::mem::size_of::<GiTraceParamsGpu>() == 80);

/// Per-dispatch inputs in CPU form.
#[derive(Clone, Copy, Debug)]
pub struct GiTraceParams {
    /// Position of probe `(0, 0, 0)`.
    pub grid_origin: Vec3,
    /// Distance between neighbouring probes in metres.
    pub spacing: f32,
    /// Probe counts per axis.
    pub dims: [u32; 3],
    /// Rays per probe, clamped to `1..=MAX_RAYS_PER_PROBE`.
    pub rays: u32,
    /// Frame index; the ray directions are rotated by a hash of it.
    pub frame_index: u32,
    /// Unit direction towards the sun (zero disables the sun term).
    pub sun_dir: Vec3,
    /// Sun radiance.
    pub sun_radiance: Vec3,
}

/// Compute pipeline and buffers for the probe ray trace.
pub struct GiTracer {
    pipeline: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    sampler: wgpu::Sampler,
    /// `vec4<f32>` per ray, `max_probes * MAX_RAYS_PER_PROBE` records.
    pub ray_data: wgpu::Buffer,
    max_probes: u32,
}

impl GiTracer {
    /// Creates the pipeline and a ray buffer for up to `max_probes` probes.
    pub fn new(device: &wgpu::Device, max_probes: u32) -> Self {
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gi_trace_bgl"),
            entries: &[
                uniform_entry(0),
                texture_entry(1, wgpu::TextureViewDimension::D3),
                texture_entry(2, wgpu::TextureViewDimension::D3),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture_entry(4, wgpu::TextureViewDimension::Cube),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                uniform_entry(6),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gi_trace_layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let shader = create_shader_module(device, "gi_trace", GI_TRACE_WGSL);
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            cache: crate::pipeline_cache::get(),
            label: Some("gi_trace_pipeline"),
            layout: Some(&layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gi_trace_params"),
            size: std::mem::size_of::<GiTraceParamsGpu>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gi_trace_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let ray_data = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gi_trace_ray_data"),
            size: u64::from(max_probes.max(1)) * u64::from(MAX_RAYS_PER_PROBE) * RAY_RECORD_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            bgl,
            params,
            sampler,
            ray_data,
            max_probes,
        }
    }

    /// Records the trace for one frame. `sky` is the environment cube sampled by missed rays.
    ///
    /// The probe count `dims.x * dims.y * dims.z` must not exceed the `max_probes` given to
    /// [`Self::new`].
    pub fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        params: &GiTraceParams,
        sdf: &GlobalSdfGpu,
        sky: &wgpu::TextureView,
    ) {
        let probes = params.dims[0] * params.dims[1] * params.dims[2];
        debug_assert!(
            probes <= self.max_probes,
            "more probes than the ray buffer holds"
        );
        let rays = params.rays.clamp(1, MAX_RAYS_PER_PROBE);
        let gpu = GiTraceParamsGpu {
            grid_origin: [
                params.grid_origin.x,
                params.grid_origin.y,
                params.grid_origin.z,
                params.spacing,
            ],
            grid_dims: [params.dims[0], params.dims[1], params.dims[2], rays],
            sun_dir: [params.sun_dir.x, params.sun_dir.y, params.sun_dir.z, 0.0],
            sun_rad: [
                params.sun_radiance.x,
                params.sun_radiance.y,
                params.sun_radiance.z,
                0.0,
            ],
            frame: [params.frame_index, probes, 0, 0],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&gpu));

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gi_trace_bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&sdf.distance_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&sdf.albedo_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(sky),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.ray_data.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: sdf.info.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("gi_trace"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups((probes * rays).div_ceil(64).max(1), 1, 1);
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

fn texture_entry(
    binding: u32,
    dimension: wgpu::TextureViewDimension,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: dimension,
            multisampled: false,
        },
        count: None,
    }
}
