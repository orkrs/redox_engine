//! Probe atlases: irradiance and visibility per probe, one octahedral tile each, in ping-pong
//! `Rgba16Float` textures (GI-01 step 4).
//!
//! Tile layout: an inner tile of `n` texels per side plus a one-texel border, so the tile
//! stride is `n + 2`. Probe `i` sits at column `i % probes_per_row`, row `i / probes_per_row`.
//! Irradiance uses `n = 8` (stride 10), visibility `n = 16` (stride 18). The visibility
//! texture is `Rgba16Float` rather than `Rg16Float` because `Rg16Float` is not storage-capable.
//! Irradiance is stored as `accum^(1/5)`; visibility as `(mean t, mean t^2)`.
//! The WGSL side is `shaders/gi_update.wgsl`; the ray records come from [`crate::gi::trace`].

use bytemuck::{Pod, Zeroable};
use redox_math::Vec3;

use crate::gi::oct::oct_encode;
use crate::gi::trace::MAX_RAYS_PER_PROBE;
use crate::shader::{GI_UPDATE_WGSL, create_shader_module};

/// Inner texels per side of an irradiance tile.
pub const IRRADIANCE_INNER: u32 = 8;
/// Inner texels per side of a visibility tile.
pub const VISIBILITY_INNER: u32 = 16;
/// Border texels on each side of a tile.
pub const TILE_BORDER: u32 = 1;
/// Exponent of the perceptual irradiance encoding (stored value = accum^(1/5)).
pub const PERCEPTUAL_POWER: f32 = 5.0;
/// Fraction of the maximum probe distance at which visibility distances are clamped.
pub const VISIBILITY_CLAMP: f32 = 0.75;
/// Old-value weight of a settled atlas.
pub const HYSTERESIS_STABLE: f32 = 0.97;
/// Old-value weight after a lighting change.
pub const HYSTERESIS_CHANGED: f32 = 0.8;
/// Old-value weight after the sun moved by more than 2 degrees.
pub const HYSTERESIS_SUN: f32 = 0.9;
/// Frames that use the changed hysteresis after a lighting change.
pub const CHANGE_FRAMES: u32 = 8;

/// Hysteresis for a frame, `frames_since_change` frames after a lighting change. `sun_moved`
/// selects [`HYSTERESIS_SUN`] over [`HYSTERESIS_CHANGED`].
pub fn hysteresis_after_change(frames_since_change: u32, sun_moved: bool) -> f32 {
    if frames_since_change >= CHANGE_FRAMES {
        HYSTERESIS_STABLE
    } else if sun_moved {
        HYSTERESIS_SUN
    } else {
        HYSTERESIS_CHANGED
    }
}

/// Update parameters mirrored by `struct UpdateParams` in `gi_update.wgsl` (32 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct AtlasUpdateGpu {
    /// Probes per row (x), probe count (y), rays per probe (z), frame index (w).
    pub layout: [u32; 4],
    /// Old-value weight (x), visibility distance clamp (y), unused (zw).
    pub blend: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<AtlasUpdateGpu>() == 32);

/// Per-frame inputs of one atlas update, in CPU form.
#[derive(Clone, Copy, Debug)]
pub struct AtlasUpdate {
    /// Rays per probe as traced; clamped to `1..=MAX_RAYS_PER_PROBE`.
    pub rays: u32,
    /// Frame index the trace used for its ray rotation.
    pub frame_index: u32,
    /// Old-value weight, usually [`HYSTERESIS_STABLE`].
    pub hysteresis: f32,
    /// Maximum probe distance; visibility distances are clamped to 0.75 of it.
    pub max_probe_distance: f32,
}

/// Tile arrangement of a set of probes in one atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtlasLayout {
    /// Number of probes stored.
    pub probe_count: u32,
    /// Probes per atlas row, `ceil(sqrt(probe_count))` (at least 1).
    pub probes_per_row: u32,
    /// Inner texels per side of a tile.
    pub tile_inner: u32,
}

impl AtlasLayout {
    /// Layout for `probe_count` probes with tiles of `tile_inner` texels per side.
    pub fn new(probe_count: u32, tile_inner: u32) -> Self {
        Self {
            probe_count,
            probes_per_row: probes_per_row(probe_count),
            tile_inner,
        }
    }

    /// Tile stride including the border.
    pub fn stride(&self) -> u32 {
        self.tile_inner + 2 * TILE_BORDER
    }

    /// Number of tile rows.
    pub fn rows(&self) -> u32 {
        self.probe_count.div_ceil(self.probes_per_row).max(1)
    }

    /// Texture size in texels.
    pub fn size(&self) -> [u32; 2] {
        [
            self.probes_per_row * self.stride(),
            self.rows() * self.stride(),
        ]
    }

    /// Texel of the tile's top-left corner (border included).
    pub fn texel_origin(&self, probe: u32) -> [u32; 2] {
        let stride = self.stride();
        [
            (probe % self.probes_per_row) * stride,
            (probe / self.probes_per_row) * stride,
        ]
    }

    /// Atlas uv (in `[0, 1]²`) of direction `dir` in probe `probe`'s inner tile.
    pub fn uv_for(&self, probe: u32, dir: Vec3) -> [f32; 2] {
        let oct = oct_encode(dir);
        let origin = self.texel_origin(probe);
        let size = self.size();
        let inner = self.tile_inner as f32;
        // Texel edges: the inner tile spans `origin + 1 ..= origin + 1 + inner`.
        let x = origin[0] as f32 + TILE_BORDER as f32 + oct.x * inner;
        let y = origin[1] as f32 + TILE_BORDER as f32 + oct.y * inner;
        [x / size[0] as f32, y / size[1] as f32]
    }
}

/// `ceil(sqrt(count))` computed in integers, at least 1.
pub fn probes_per_row(count: u32) -> u32 {
    let mut side = (f64::from(count)).sqrt() as u32;
    while u64::from(side) * u64::from(side) < u64::from(count) {
        side += 1;
    }
    side.max(1)
}

/// A texture pair: `current` is readable, the other is the write target of the next update.
struct PingPong {
    textures: [wgpu::Texture; 2],
    views: [wgpu::TextureView; 2],
}

impl PingPong {
    fn new(device: &wgpu::Device, label: &str, size: [u32; 2]) -> Self {
        let make = || {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size[0].max(1),
                    height: size[1].max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let textures = [make(), make()];
        let views = [
            textures[0].create_view(&Default::default()),
            textures[1].create_view(&Default::default()),
        ];
        Self { textures, views }
    }
}

/// Irradiance and visibility atlases with their update pipelines.
pub struct ProbeAtlas {
    irradiance_layout: AtlasLayout,
    visibility_layout: AtlasLayout,
    irradiance: PingPong,
    visibility: PingPong,
    /// Index of the readable (current) texture in both pairs.
    current: usize,
    bgl: wgpu::BindGroupLayout,
    irradiance_pipeline: wgpu::ComputePipeline,
    visibility_pipeline: wgpu::ComputePipeline,
    params: wgpu::Buffer,
}

impl ProbeAtlas {
    /// Atlases for `probe_count` probes, all zero.
    pub fn new(device: &wgpu::Device, probe_count: u32) -> Self {
        let irradiance_layout = AtlasLayout::new(probe_count, IRRADIANCE_INNER);
        let visibility_layout = AtlasLayout::new(probe_count, VISIBILITY_INNER);
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gi_update_bgl"),
            entries: &[
                uniform_entry(0),
                texture_2d_entry(1),
                texture_2d_entry(2),
                storage_texture_entry(3),
                storage_texture_entry(4),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gi_update_layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let module = create_shader_module(device, "gi_update", GI_UPDATE_WGSL);
        let pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                cache: crate::pipeline_cache::get(),
                label: Some(entry),
                layout: Some(&layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
            })
        };
        let irradiance_pipeline = pipeline("update_irradiance");
        let visibility_pipeline = pipeline("update_visibility");
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gi_update_params"),
            size: std::mem::size_of::<AtlasUpdateGpu>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            irradiance: PingPong::new(device, "gi_irradiance_atlas", irradiance_layout.size()),
            visibility: PingPong::new(device, "gi_visibility_atlas", visibility_layout.size()),
            irradiance_layout,
            visibility_layout,
            current: 0,
            bgl,
            irradiance_pipeline,
            visibility_pipeline,
            params,
        }
    }

    /// Readable irradiance atlas (the result of the last update).
    pub fn irradiance(&self) -> &wgpu::Texture {
        &self.irradiance.textures[self.current]
    }

    /// Readable visibility atlas (the result of the last update).
    pub fn visibility(&self) -> &wgpu::Texture {
        &self.visibility.textures[self.current]
    }

    /// Layout of the irradiance atlas.
    pub fn irradiance_layout(&self) -> AtlasLayout {
        self.irradiance_layout
    }

    /// Layout of the visibility atlas.
    pub fn visibility_layout(&self) -> AtlasLayout {
        self.visibility_layout
    }

    /// Records one update: irradiance and visibility from `ray_data`, then swaps the textures.
    /// `ray_data` must hold `probe_count * rays` records as written by the GI trace.
    pub fn record_update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        update: &AtlasUpdate,
        ray_data: &wgpu::Buffer,
    ) {
        let probes = self.irradiance_layout.probe_count;
        if probes == 0 {
            return;
        }
        let rays = update.rays.clamp(1, MAX_RAYS_PER_PROBE);
        let gpu = AtlasUpdateGpu {
            layout: [
                self.irradiance_layout.probes_per_row,
                probes,
                rays,
                update.frame_index,
            ],
            blend: [
                update.hysteresis,
                VISIBILITY_CLAMP * update.max_probe_distance,
                0.0,
                0.0,
            ],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&gpu));

        let read = self.current;
        let write = 1 - read;
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gi_update_bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.irradiance.views[read]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.visibility.views[read]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&self.irradiance.views[write]),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&self.visibility.views[write]),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: ray_data.as_entire_binding(),
                },
            ],
        });
        // One workgroup per tile: x = column, y = row (probe = y * probes_per_row + x).
        let groups = self.irradiance_layout.probes_per_row;
        let rows = self.irradiance_layout.rows();
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gi_update_irradiance"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.irradiance_pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(groups, rows, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gi_update_visibility"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.visibility_pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(groups, rows, 1);
        }
        self.current = write;
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

fn texture_2d_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn storage_texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::Rgba16Float,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}
