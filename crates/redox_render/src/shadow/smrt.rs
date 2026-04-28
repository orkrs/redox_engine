//! SMRT (Shadow Map Ray Tracing) configuration.
//!
//! SMRT produces contact-hardening soft shadows by tracing multiple rays
//! from the shading point towards the light source and sampling the virtual
//! shadow map at several positions along each ray.

use bytemuck::{Pod, Zeroable};

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

impl Default for SmrtConfig {
    fn default() -> Self {
        Self {
            ray_count: 4,
            samples_per_ray: 8,
            source_angle: 0.0087,
            source_radius: 0.1,
            max_trace_dist: 50.0,
        }
    }
}

/// GPU-uploadable SMRT uniform (appended to the VSM info buffer).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SmrtUniform {
    pub ray_count: u32,
    pub samples_per_ray: u32,
    pub source_angle: f32,
    pub source_radius: f32,
    pub max_trace_dist: f32,
    pub _pad: [f32; 3],
}

impl SmrtUniform {
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
