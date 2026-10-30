//! GPU probe ray trace against the global SDF (`shaders/gi_trace.wgsl`).
//!
//! Each probe casts `rays` rays; results land in `ray_data` as `(radiance, hit distance)`
//! at index `probe * rays + ray`. A hit is shaded with emission, direct sun and the bounce term:
//! last frame's irradiance atlas at the hit point, times albedo / pi.

use bytemuck::{Pod, Zeroable};
use redox_math::Vec3;

use super::card_atlas::{CardAtlas, CardObjectGpu};
use crate::sdf::global_gpu::GlobalSdfGpu;
use crate::shader::{GI_TRACE_WGSL, create_shader_module};

/// Largest ray count per probe; sizes the ray buffer.
pub const MAX_RAYS_PER_PROBE: u32 = 128;
/// Bytes per ray record: `vec4<f32>`.
const RAY_RECORD_BYTES: u64 = 16;

/// Card switch mirrored by `struct CardTraceParams` in `gi_trace.wgsl` (16 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct CardTraceParamsGpu {
    /// x: 1 when cards are read; y: table length.
    enabled: [u32; 4],
}

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
    /// Frame index (x), probes in this dispatch (y), first probe of the atlas (z).
    pub frame: [u32; 4],
    /// Lowest world cell of the cascade window (xyz).
    pub scroll: [i32; 4],
}

const _: () = assert!(std::mem::size_of::<GiTraceParamsGpu>() == 96);

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
    /// Lowest world cell of the cascade window (see `ProbeCascade::scroll`).
    pub scroll: [i32; 3],
    /// Atlas index of this dispatch's first probe; ray records start at `probe_base * rays`.
    pub probe_base: u32,
}

/// Number of parameter slots: one per cascade, so each dispatch reads its own uniform.
pub const TRACE_SLOTS: usize = 2;

/// Resources a trace reads besides its parameters.
pub struct TraceInputs<'a> {
    /// Global SDF the rays march through.
    pub sdf: &'a GlobalSdfGpu,
    /// Environment cube sampled by rays that escape the volume.
    pub sky: &'a wgpu::TextureView,
    /// Probe grid of the previous frame, matching `prev_irradiance` (`GiGridGpu`).
    pub prev_grid: &'a wgpu::Buffer,
    /// Irradiance atlas of the previous frame (stable copy).
    pub prev_irradiance: &'a wgpu::TextureView,
    /// Relocated probe offsets, one `vec4` per atlas index.
    pub probe_offset: &'a wgpu::Buffer,
    /// Surface cards read at hit points; `None` traces the probe path only (GI-01).
    pub cards: Option<&'a CardAtlas>,
}

/// Compute pipeline and buffers for the probe ray trace.
pub struct GiTracer {
    pipeline: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
    params: [wgpu::Buffer; TRACE_SLOTS],
    sampler: wgpu::Sampler,
    card_params: wgpu::Buffer,
    /// Stand-ins bound when no cards are read (the shader reads them only when enabled).
    dummy_radiance: wgpu::TextureView,
    dummy_normal_depth: wgpu::TextureView,
    dummy_table: wgpu::Buffer,
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
                uniform_entry(7),
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture_entry(10, wgpu::TextureViewDimension::D2),
                texture_entry(11, wgpu::TextureViewDimension::D2),
                wgpu::BindGroupLayoutEntry {
                    binding: 12,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 13,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                uniform_entry(14),
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
        let params = std::array::from_fn(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gi_trace_params"),
                size: std::mem::size_of::<GiTraceParamsGpu>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
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
        let card_params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gi_trace_card_params"),
            size: std::mem::size_of::<CardTraceParamsGpu>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let dummy_texture = |label: &str, format: wgpu::TextureFormat| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let dummy_radiance =
            dummy_texture("gi_trace_dummy_radiance", wgpu::TextureFormat::Rgba16Float);
        let dummy_normal_depth = dummy_texture(
            "gi_trace_dummy_normal_depth",
            wgpu::TextureFormat::Rgba8Unorm,
        );
        let dummy_table = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gi_trace_dummy_table"),
            size: std::mem::size_of::<CardObjectGpu>() as u64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
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
            card_params,
            dummy_radiance,
            dummy_normal_depth,
            dummy_table,
            ray_data,
            max_probes,
        }
    }

    /// Records the trace for one frame in parameter slot 0. See [`Self::record_slot`].
    ///
    /// The probe count `dims.x * dims.y * dims.z` must not exceed the `max_probes` given to
    /// [`Self::new`].
    pub fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        params: &GiTraceParams,
        inputs: &TraceInputs<'_>,
    ) {
        self.record_slot(device, queue, encoder, params, 0, inputs);
    }

    /// Records one dispatch with parameter slot `slot` (`< TRACE_SLOTS`).
    ///
    /// Each dispatch of one frame needs its own slot: `queue.write_buffer` takes effect before
    /// the whole encoder runs, so dispatches sharing one uniform would all read the last write.
    /// Ray records go to `(probe_base + probe) * rays + ray`.
    pub fn record_slot(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        params: &GiTraceParams,
        slot: usize,
        inputs: &TraceInputs<'_>,
    ) {
        let probes = params.dims[0] * params.dims[1] * params.dims[2];
        debug_assert!(
            params.probe_base + probes <= self.max_probes,
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
            frame: [params.frame_index, probes, params.probe_base, 0],
            scroll: [params.scroll[0], params.scroll[1], params.scroll[2], 0],
        };
        let buffer = &self.params[slot.min(TRACE_SLOTS - 1)];
        queue.write_buffer(buffer, 0, bytemuck::bytes_of(&gpu));
        let cards = inputs.cards;
        let card_switch = CardTraceParamsGpu {
            enabled: [
                u32::from(cards.is_some()),
                cards.map_or(0, |c| c.table_len),
                0,
                0,
            ],
        };
        queue.write_buffer(&self.card_params, 0, bytemuck::bytes_of(&card_switch));
        let (radiance, normal_depth, table) = match cards {
            Some(c) => (&c.radiance_view, &c.normal_depth_view, &c.table),
            None => (
                &self.dummy_radiance,
                &self.dummy_normal_depth,
                &self.dummy_table,
            ),
        };

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gi_trace_bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&inputs.sdf.distance_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&inputs.sdf.albedo_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(inputs.sky),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.ray_data.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: inputs.sdf.info.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: inputs.prev_grid.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(inputs.prev_irradiance),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: inputs.probe_offset.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::TextureView(radiance),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: wgpu::BindingResource::TextureView(normal_depth),
                },
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: table.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 13,
                    resource: wgpu::BindingResource::TextureView(&inputs.sdf.object_id_view),
                },
                wgpu::BindGroupEntry {
                    binding: 14,
                    resource: self.card_params.as_entire_binding(),
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
