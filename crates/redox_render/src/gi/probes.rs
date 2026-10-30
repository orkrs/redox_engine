//! Renderer-side probe set for GI: two cascades that follow the camera, relocated out of
//! geometry, traced against the global SDF, filtered into the irradiance and visibility atlases.
//!
//! The atlases ping-pong inside [`ProbeAtlas`]. Their readable result is copied every frame into
//! two textures with fixed views, which the scene bind group reads (`@binding(17)`, `@binding(23)`),
//! so the bind group never has to be rebuilt.

use redox_math::Vec3;

use super::atlas::{AtlasUpdate, ProbeAtlas, hysteresis_after_change};
use super::cascade::ProbeCascade;
use super::grid_gpu::{GI_CASCADE_COUNT, GiGridGpu};
use super::relocate::GiRelocator;
use super::trace::{GiTraceParams, GiTracer, TRACE_SLOTS, TraceInputs};
use crate::sdf::global_gpu::GlobalSdfGpu;

/// Probes per cascade along each axis.
pub const CASCADE_DIMS: [u32; 3] = [16, 8, 16];
/// Probe spacing of the fine cascade, metres.
pub const FINE_SPACING: f32 = 0.5;
/// Probe spacing of the coarse cascade, metres.
pub const COARSE_SPACING: f32 = 2.0;
/// Longest ray distance; visibility distances are clamped below it.
pub const MAX_PROBE_DISTANCE: f32 = 40.0;
/// Probes in one cascade.
const PROBES_PER_CASCADE: u32 = CASCADE_DIMS[0] * CASCADE_DIMS[1] * CASCADE_DIMS[2];
/// Sun direction change, as the cosine of 2 degrees, that restarts the lighting hysteresis.
const SUN_MOVE_COS: f32 = 0.999_390_8;

/// Quality of the probe trace. `Off` traces nothing and the frame is as before GI.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GiQuality {
    /// No probes are traced or read.
    Off,
    /// 32 rays per probe, one cascade.
    Low,
    /// 64 rays per probe, two cascades.
    #[default]
    Medium,
    /// 128 rays per probe, two cascades.
    High,
}

impl GiQuality {
    /// Rays per probe per frame.
    pub fn rays(self) -> u32 {
        match self {
            Self::Off => 0,
            Self::Low => 32,
            Self::Medium => 64,
            Self::High => 128,
        }
    }

    /// Cascades traced and read.
    pub fn cascades(self) -> u32 {
        match self {
            Self::Low => 1,
            Self::Off | Self::Medium | Self::High => GI_CASCADE_COUNT as u32,
        }
    }
}

/// Per-frame inputs of the GI update.
pub struct GiFrame<'a> {
    /// Camera position; the cascades are centred on it.
    pub camera_pos: Vec3,
    /// Unit direction towards the sun (zero disables the sun term).
    pub sun_dir: Vec3,
    /// Sun radiance.
    pub sun_radiance: Vec3,
    /// Environment cube sampled by rays that escape the volume.
    pub sky: &'a wgpu::TextureView,
    /// Global SDF the rays march through.
    pub sdf: &'a GlobalSdfGpu,
}

/// Both probe cascades with their atlases, trace and the stable textures the shader reads.
pub struct GiProbes {
    cascades: [ProbeCascade; GI_CASCADE_COUNT],
    atlas: ProbeAtlas,
    tracer: GiTracer,
    relocator: GiRelocator,
    grid_buffer: wgpu::Buffer,
    /// Grid of the previous frame, matching the stable atlas the trace reads (bounce term).
    prev_grid: GiGridGpu,
    prev_grid_buffer: wgpu::Buffer,
    /// Relocated probe offsets, one `vec4` per atlas index.
    probe_offset: wgpu::Buffer,
    irradiance: wgpu::Texture,
    irradiance_view: wgpu::TextureView,
    visibility: wgpu::Texture,
    visibility_view: wgpu::TextureView,
    frame_index: u32,
    /// Frames since the last lighting change or cascade move; selects the hysteresis.
    frames_since_change: u32,
    /// True when the last change was a sun move (hysteresis 0.9 instead of 0.8).
    sun_changed: bool,
    /// Sun direction of the previous enabled frame; zero when there was none.
    last_sun_dir: Vec3,
    /// Trace and shading quality.
    quality: GiQuality,
    /// Off until a static scene is given: with an empty volume every ray would return the sky and
    /// double the ambient light of scenes that do not use GI.
    enabled: bool,
}

impl GiProbes {
    /// Allocates the cascades, atlases and stable textures. Everything starts at zero.
    pub fn new(device: &wgpu::Device) -> Self {
        let total = PROBES_PER_CASCADE * GI_CASCADE_COUNT as u32;
        let atlas = ProbeAtlas::new(device, total);
        let irradiance_size = atlas.irradiance_layout().size();
        let visibility_size = atlas.visibility_layout().size();
        let irradiance = stable_texture(device, "gi_irradiance_stable", irradiance_size);
        let visibility = stable_texture(device, "gi_visibility_stable", visibility_size);
        let grid_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gi_grid_uniform"),
            size: std::mem::size_of::<GiGridGpu>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let prev_grid_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gi_prev_grid_uniform"),
            size: std::mem::size_of::<GiGridGpu>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let probe_offset = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gi_probe_offset"),
            size: u64::from(total) * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            cascades: [
                ProbeCascade::new(Vec3::ZERO, FINE_SPACING, CASCADE_DIMS),
                ProbeCascade::new(Vec3::ZERO, COARSE_SPACING, CASCADE_DIMS),
            ],
            tracer: GiTracer::new(device, total),
            relocator: GiRelocator::new(device),
            atlas,
            grid_buffer,
            prev_grid: GiGridGpu::default(),
            prev_grid_buffer,
            probe_offset,
            irradiance_view: irradiance.create_view(&Default::default()),
            irradiance,
            visibility_view: visibility.create_view(&Default::default()),
            visibility,
            frame_index: 0,
            // A fresh atlas is blended with the short hysteresis until it has filled in.
            frames_since_change: 0,
            sun_changed: false,
            last_sun_dir: Vec3::ZERO,
            quality: GiQuality::default(),
            enabled: false,
        }
    }

    /// Switches the probes on or off. Off, they are neither traced nor read (the frame is as before GI).
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.frames_since_change = 0;
    }

    /// Sets the trace quality. Changing it restarts the hysteresis.
    pub fn set_quality(&mut self, quality: GiQuality) {
        self.quality = quality;
        self.frames_since_change = 0;
    }

    /// Current trace quality.
    pub fn quality(&self) -> GiQuality {
        self.quality
    }

    /// Whether the probes are traced and read: enabled and not at `Off` quality.
    pub fn active(&self) -> bool {
        self.enabled && self.quality != GiQuality::Off
    }

    /// Uniform buffer with the cascade description (`@binding(16)`).
    pub fn grid_buffer(&self) -> &wgpu::Buffer {
        &self.grid_buffer
    }

    /// Stable irradiance atlas view (`@binding(17)`).
    pub fn irradiance_view(&self) -> &wgpu::TextureView {
        &self.irradiance_view
    }

    /// Stable visibility atlas view (`@binding(23)`).
    pub fn visibility_view(&self) -> &wgpu::TextureView {
        &self.visibility_view
    }

    /// Relocated probe offsets (`vec4` per atlas index), read by the shading and the trace.
    pub fn probe_offset(&self) -> &wgpu::Buffer {
        &self.probe_offset
    }

    /// Current cascades, fine first.
    pub fn cascades(&self) -> &[ProbeCascade; GI_CASCADE_COUNT] {
        &self.cascades
    }

    /// Marks a lighting or scene change; the next frames blend with the shorter hysteresis.
    pub fn notify_lighting_changed(&mut self) {
        self.frames_since_change = 0;
        self.sun_changed = false;
    }

    /// Moves the cascades to the camera, relocates the probes, traces, updates the atlases and
    /// refreshes the stable textures and the grid uniform. Records into `encoder`.
    ///
    /// The trace reads last frame's grid and stable atlas (the bounce term), so `prev_grid` is
    /// uploaded before this frame's grid replaces it.
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &GiFrame<'_>,
    ) {
        queue.write_buffer(&self.prev_grid_buffer, 0, bytemuck::bytes_of(&self.prev_grid));
        let layout = self.atlas.irradiance_layout();
        let cascade_count = self.quality.cascades();
        if !self.active() {
            let grid = GiGridGpu::new(&self.cascades, layout.probes_per_row, false, cascade_count);
            queue.write_buffer(&self.grid_buffer, 0, bytemuck::bytes_of(&grid));
            self.prev_grid = grid;
            return;
        }
        if self.snap_cascades(frame.camera_pos) {
            self.frames_since_change = 0;
            self.sun_changed = false;
        }
        if self.sun_moved(frame.sun_dir) {
            self.frames_since_change = 0;
            self.sun_changed = true;
        }
        self.last_sun_dir = frame.sun_dir;
        let hysteresis = hysteresis_after_change(self.frames_since_change, self.sun_changed);
        self.frames_since_change = self.frames_since_change.saturating_add(1);

        let grid = GiGridGpu::new(&self.cascades, layout.probes_per_row, true, cascade_count);
        queue.write_buffer(&self.grid_buffer, 0, bytemuck::bytes_of(&grid));
        self.relocator.record(
            device,
            encoder,
            &self.grid_buffer,
            frame.sdf,
            &self.probe_offset,
            PROBES_PER_CASCADE * cascade_count,
        );

        let rays = self.quality.rays();
        let inputs = TraceInputs {
            sdf: frame.sdf,
            sky: frame.sky,
            prev_grid: &self.prev_grid_buffer,
            prev_irradiance: &self.irradiance_view,
            probe_offset: &self.probe_offset,
        };
        for (slot, cascade) in self.cascades.iter().enumerate().take(cascade_count as usize) {
            let params = GiTraceParams {
                grid_origin: cascade.origin,
                spacing: cascade.spacing,
                dims: cascade.dims,
                rays,
                frame_index: self.frame_index,
                sun_dir: frame.sun_dir,
                sun_radiance: frame.sun_radiance,
                scroll: cascade.scroll,
                probe_base: slot as u32 * PROBES_PER_CASCADE,
            };
            self.tracer
                .record_slot(device, queue, encoder, &params, slot, &inputs);
        }

        self.atlas.record_update(
            device,
            queue,
            encoder,
            &AtlasUpdate {
                rays,
                frame_index: self.frame_index,
                hysteresis,
                max_probe_distance: MAX_PROBE_DISTANCE,
            },
            &self.tracer.ray_data,
        );
        self.copy_readable_atlases(encoder);
        self.frame_index = self.frame_index.wrapping_add(1);
        self.prev_grid = grid;
    }

    /// True when the sun moved more than 2 degrees since the previous enabled frame. A sun that
    /// was absent (zero direction) counts as not moved.
    fn sun_moved(&self, sun_dir: Vec3) -> bool {
        let had_sun = self.last_sun_dir.length() > 0.0;
        let has_sun = sun_dir.length() > 0.0;
        had_sun && has_sun && self.last_sun_dir.dot(sun_dir) < SUN_MOVE_COS
    }

    /// Re-centres both cascades on `camera_pos`. Returns true when any cascade moved.
    fn snap_cascades(&mut self, camera_pos: Vec3) -> bool {
        let mut moved = false;
        for cascade in &mut self.cascades {
            let shift = cascade.snap_to_camera(camera_pos);
            moved |= shift != [0; 3];
        }
        moved
    }

    /// Copies the atlas result of this frame into the stable textures.
    fn copy_readable_atlases(&self, encoder: &mut wgpu::CommandEncoder) {
        let irr = self.atlas.irradiance_layout().size();
        let vis = self.atlas.visibility_layout().size();
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: self.atlas.irradiance(),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &self.irradiance,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            extent(irr),
        );
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: self.atlas.visibility(),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &self.visibility,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            extent(vis),
        );
    }
}

fn extent(size: [u32; 2]) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: size[0].max(1),
        height: size[1].max(1),
        depth_or_array_layers: 1,
    }
}

fn stable_texture(device: &wgpu::Device, label: &str, size: [u32; 2]) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: extent(size),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

// One trace parameter slot per cascade.
const _: () = assert!(TRACE_SLOTS == GI_CASCADE_COUNT);
