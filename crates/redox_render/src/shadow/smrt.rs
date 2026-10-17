//! SMRT (Shadow Map Ray Tracing) configuration.
//!
//! SMRT produces contact-hardening soft shadows by tracing multiple rays
//! from the shading point towards the light source and sampling the virtual
//! shadow map at several positions along each ray.

use bytemuck::{Pod, Zeroable};

/// User-facing SMRT quality preset (maps to ray/sample counts).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SmrtQuality {
    /// 4×5 — default; contact OK, far penumbra noisier.
    #[default]
    Performance,
    /// 6×8 — 1080p quality tradeoff.
    Balanced,
    /// 12×10 — showcase / screenshots.
    Quality,
}

impl SmrtQuality {
    /// Ray-tracing configuration of this quality level.
    pub fn config(self) -> SmrtConfig {
        match self {
            Self::Performance => SmrtConfig {
                ray_count: 4,
                samples_per_ray: 5,
                source_angle: 0.013,
                max_trace_dist: 16.0,
                ..SmrtConfig::default_fields()
            },
            Self::Balanced => SmrtConfig {
                ray_count: 6,
                samples_per_ray: 8,
                source_angle: 0.015,
                ..SmrtConfig::default_fields()
            },
            Self::Quality => SmrtConfig {
                ray_count: 16,
                samples_per_ray: 12,
                source_angle: 0.05,
                ..SmrtConfig::default_fields()
            },
        }
    }
}

/// CPU-side SMRT configuration (subset of `VsmConfig`).
#[derive(Clone, Debug)]
pub struct SmrtConfig {
    /// Number of rays per pixel.
    pub ray_count: u32,
    /// Shadow-map samples per ray.
    pub samples_per_ray: u32,
    /// Angular radius of the directional light source (radians).
    /// The default approximates the Sun: ~0.53° ≈ 0.0087 rad.
    pub source_angle: f32,
    /// Physical radius for local (point/spot) lights.
    pub source_radius: f32,
    /// Maximum world-space distance a ray can travel.
    pub max_trace_dist: f32,
}

impl SmrtConfig {
    fn default_fields() -> Self {
        Self {
            ray_count: 6,
            samples_per_ray: 8,
            source_angle: 0.015,
            source_radius: 0.1,
            max_trace_dist: 24.0,
        }
    }
}

impl Default for SmrtConfig {
    fn default() -> Self {
        SmrtQuality::Performance.config()
    }
}

/// GPU-uploadable SMRT uniform (appended to the VSM info buffer).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SmrtUniform {
    /// Number of shadow rays per pixel.
    pub ray_count: u32,
    /// Depth samples per ray.
    pub samples_per_ray: u32,
    /// Angular size of the light source in radians.
    pub source_angle: f32,
    /// Radius of the light source.
    pub source_radius: f32,
    /// Maximum distance a ray travels.
    pub max_trace_dist: f32,
    /// Padding.
    pub _pad: [f32; 3],
}

/// Builds the GPU uniform from a config (free-function form of
/// [`SmrtUniform::from_config`]).
pub fn smrt_sample_params(cfg: &SmrtConfig) -> SmrtUniform {
    SmrtUniform::from_config(cfg)
}

impl SmrtUniform {
    /// Converts a configuration to the GPU layout.
    pub fn from_config(cfg: &SmrtConfig) -> Self {
        Self {
            ray_count: cfg.ray_count,
            samples_per_ray: cfg.samples_per_ray,
            source_angle: cfg.source_angle,
            source_radius: cfg.source_radius,
            max_trace_dist: cfg.max_trace_dist,
            _pad: [0.0; 3],
        }
    }
}
