//! Card light pass (GI-02 step 3): lights a quarter of the card atlas each frame
//! (`shaders/gi_card_light.wgsl`).

use bytemuck::{Pod, Zeroable};
use redox_math::Vec3;

use super::card_atlas::CardAtlas;
use super::cards::CARD_FACES;
use crate::sdf::global_gpu::GlobalSdfGpu;
use crate::shader::{GI_CARD_LIGHT_WGSL, create_shader_module};

/// Point lights the pass considers per frame (the light uniform holds at most this many).
pub const MAX_CARD_POINTS: usize = 128;
/// Workgroup edge of `gi_card_light.wgsl`.
const WORKGROUP_EDGE: u32 = 8;

/// Mirrors `struct CardLightParams` in `gi_card_light.wgsl` (48 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CardLightParamsGpu {
    /// Unit direction towards the sun (xyz).
    pub sun_dir: [f32; 4],
    /// Sun radiance (xyz).
    pub sun_rad: [f32; 4],
    /// Frame index (x), cards table length (y), point count (z).
    pub frame: [u32; 4],
}

const _: () = assert!(std::mem::size_of::<CardLightParamsGpu>() == 48);

/// Mirrors `struct CardPoint` in `gi_card_light.wgsl` (32 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct CardPointGpu {
    /// Position (xyz) and intensity (w).
    pub pos_intensity: [f32; 4],
    /// Colour (xyz) and radius (w).
    pub color_radius: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<CardPointGpu>() == 32);

/// Resources the light pass reads besides the atlas, all owned by the caller.
pub struct CardLightInputs<'a> {
    /// Global SDF for the sun visibility march.
    pub sdf: &'a GlobalSdfGpu,
    /// Probe grid of the previous frame (`GiGridGpu`).
    pub prev_grid: &'a wgpu::Buffer,
    /// Stable irradiance atlas of the previous frame.
    pub prev_irradiance: &'a wgpu::TextureView,
    /// Relocated probe offsets, one `vec4` per atlas index.
    pub probe_offset: &'a wgpu::Buffer,
    /// Unit direction towards the sun (zero disables the sun term).
    pub sun_dir: Vec3,
    /// Sun radiance.
    pub sun_radiance: Vec3,
    /// Point lights; only the first [`MAX_CARD_POINTS`] are used.
    pub points: &'a [CardPointGpu],
    /// Frame index; selects the rows lit this frame.
    pub frame_index: u32,
}

/// Compute pipeline and uniform buffers of the light pass.
pub struct CardLighter {
    pipeline: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    points: wgpu::Buffer,
    sampler: wgpu::Sampler,
}

impl CardLighter {
    /// Creates the pipeline and the parameter and point buffers.
    pub fn new(device: &wgpu::Device) -> Self {
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gi_card_light_bgl"),
            entries: &[
                uniform_entry(0),
                texture_entry(
                    1,
                    wgpu::TextureViewDimension::D3,
                    wgpu::TextureSampleType::Float { filterable: true },
                ),
                uniform_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture_entry(
                    4,
                    wgpu::TextureViewDimension::D2,
                    wgpu::TextureSampleType::Float { filterable: true },
                ),
                texture_entry(
                    5,
                    wgpu::TextureViewDimension::D2,
                    wgpu::TextureSampleType::Float { filterable: true },
                ),
                storage_entry(6, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                uniform_entry(8),
                texture_entry(
                    9,
                    wgpu::TextureViewDimension::D2,
                    wgpu::TextureSampleType::Float { filterable: true },
                ),
                storage_entry(10, true),
                storage_entry(11, true),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gi_card_light_layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let shader = create_shader_module(device, "gi_card_light", GI_CARD_LIGHT_WGSL);
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            cache: crate::pipeline_cache::get(),
            label: Some("gi_card_light_pipeline"),
            layout: Some(&layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gi_card_light_params"),
            size: std::mem::size_of::<CardLightParamsGpu>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let points = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gi_card_light_points"),
            size: (MAX_CARD_POINTS * std::mem::size_of::<CardPointGpu>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gi_card_light_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            bgl,
            params,
            points,
            sampler,
        }
    }

    /// Records one light update of `atlas`: rows `frame % 4` of every card. Does nothing while no
    /// cards are set.
    pub fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        atlas: &CardAtlas,
        inputs: &CardLightInputs<'_>,
    ) {
        if atlas.table_len == 0 || atlas.max_edge == 0 {
            return;
        }
        let points = &inputs.points[..inputs.points.len().min(MAX_CARD_POINTS)];
        let params = CardLightParamsGpu {
            sun_dir: [inputs.sun_dir.x, inputs.sun_dir.y, inputs.sun_dir.z, 0.0],
            sun_rad: [
                inputs.sun_radiance.x,
                inputs.sun_radiance.y,
                inputs.sun_radiance.z,
                0.0,
            ],
            frame: [inputs.frame_index, atlas.table_len, points.len() as u32, 0],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        if !points.is_empty() {
            queue.write_buffer(&self.points, 0, bytemuck::cast_slice(points));
        }
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gi_card_light_bg"),
            layout: &self.bgl,
            entries: &[
                bind(0, self.params.as_entire_binding()),
                bind(
                    1,
                    wgpu::BindingResource::TextureView(&inputs.sdf.distance_view),
                ),
                bind(2, inputs.sdf.info.as_entire_binding()),
                bind(3, wgpu::BindingResource::Sampler(&self.sampler)),
                bind(4, wgpu::BindingResource::TextureView(&atlas.albedo_view)),
                bind(
                    5,
                    wgpu::BindingResource::TextureView(&atlas.normal_depth_view),
                ),
                bind(6, atlas.table.as_entire_binding()),
                bind(7, wgpu::BindingResource::TextureView(&atlas.radiance_view)),
                bind(8, inputs.prev_grid.as_entire_binding()),
                bind(
                    9,
                    wgpu::BindingResource::TextureView(inputs.prev_irradiance),
                ),
                bind(10, inputs.probe_offset.as_entire_binding()),
                bind(11, self.points.as_entire_binding()),
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("gi_card_light"),
            timestamp_writes: None,
        });
        let groups = atlas.max_edge.div_ceil(WORKGROUP_EDGE).max(1);
        let slots = atlas.table_len * CARD_FACES as u32;
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(groups, groups, slots);
    }
}

fn bind(binding: u32, resource: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource }
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

fn texture_entry(
    binding: u32,
    dimension: wgpu::TextureViewDimension,
    sample_type: wgpu::TextureSampleType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: dimension,
            multisampled: false,
        },
        count: None,
    }
}
