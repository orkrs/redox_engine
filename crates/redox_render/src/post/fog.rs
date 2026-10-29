//! Height fog as a post effect, with optional in-scattering of the sun.
//!
//! Density falls off exponentially with height and the optical depth along each view ray is
//! integrated analytically. The pure functions here are mirrored in `shaders/fog.wgsl`; keep
//! both in sync. Sun in-scatter has no shadows, so light shafts show through walls too.

use bytemuck::{Pod, Zeroable};
use redox_math::Mat4;

use crate::post::post_fx::ChainFrame;

/// Height fog: density falls off exponentially with height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FogSettings {
    /// Ambient (sky) colour of the fog, linear RGB.
    pub color: [f32; 3],
    /// Extinction per metre at `base_height`. Default 0.02.
    pub density: f32,
    /// 1/m; 0 = uniform fog. Default 0.15.
    pub height_falloff: f32,
    /// World y where density equals `density`. Default 0.0.
    pub base_height: f32,
    /// No fog closer than this many metres. Default 0.0.
    pub start_distance: f32,
    /// Cap of the fog opacity, 0..1. Default 0.95.
    pub max_opacity: f32,
    /// Strength of the sun in-scatter, 0..1; 0 = off. Default 0.0.
    pub sun_scatter: f32,
    /// Henyey-Greenstein g, -0.9..0.9. Default 0.5.
    pub anisotropy: f32,
}

impl Default for FogSettings {
    fn default() -> Self {
        Self {
            color: [0.7, 0.78, 0.85],
            density: 0.02,
            height_falloff: 0.15,
            base_height: 0.0,
            start_distance: 0.0,
            max_opacity: 0.95,
            sun_scatter: 0.0,
            anisotropy: 0.5,
        }
    }
}

/// Optical depth along a ray: the integral of `density * exp(-falloff * (y - base))` over
/// `distance` metres, starting at height `y0` and rising by `dy_per_m` per metre of path.
///
/// `start_distance` is applied here: the first `start_distance` metres carry no fog and the
/// integration starts there. Falls back to `density * distance` when the falloff rate is near
/// zero, so a horizontal ray never divides by zero.
pub fn optical_depth(s: &FogSettings, y0: f32, dy_per_m: f32, distance: f32) -> f32 {
    let d = (distance - s.start_distance).max(0.0);
    let y = y0 + dy_per_m * s.start_distance;
    let density = s.density * (-s.height_falloff * (y - s.base_height)).exp();
    let k = s.height_falloff * dy_per_m;
    if k.abs() < 1e-4 {
        density * d
    } else {
        density * (1.0 - (-k * d).exp()) / k
    }
}

/// Fraction of light that reaches the eye: `exp(-optical_depth)`.
pub fn transmittance(optical_depth: f32) -> f32 {
    (-optical_depth).exp()
}

/// Henyey-Greenstein phase function for `cos_theta` between the view ray and the direction to
/// the sun. With `g = 0` it is isotropic, `1 / (4 pi)`.
pub fn hg_phase(cos_theta: f32, g: f32) -> f32 {
    let g2 = g * g;
    let denom = 1.0 + g2 - 2.0 * g * cos_theta;
    (1.0 - g2) / (4.0 * std::f32::consts::PI * denom.powf(1.5))
}

/// Depth in metres of froxel slice `i` of `n`, exponentially distributed between `near` and
/// `far`. Used by the froxel volume (not implemented yet).
pub fn slice_depth(i: u32, n: u32, near: f32, far: f32) -> f32 {
    let n = n.max(1);
    near * (far / near).powf(i as f32 / n as f32)
}

/// Uniform of `shaders/fog.wgsl`. 160 bytes; every field is a vec4 or a matrix.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct FogUniform {
    /// rgb = ambient colour, w = density.
    color_density: [f32; 4],
    /// x = height falloff, y = base height, z = start distance, w = max opacity.
    falloff: [f32; 4],
    /// x = sun scatter, y = anisotropy, z = near, w = far.
    scatter: [f32; 4],
    /// Camera world position (xyz).
    camera_pos: [f32; 4],
    /// Direction the sun shines in (xyz, unit length, zero when unknown).
    sun_dir: [f32; 4],
    /// Sun colour times intensity (rgb).
    sun_color: [f32; 4],
    /// Inverse of view-projection, for world-space view rays.
    inv_view_proj: [[f32; 4]; 4],
}

/// Unit vector of `v`, or zero when `v` is (near) zero or not finite.
fn unit_or_zero(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 1e-6 && len.is_finite() {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        [0.0; 3]
    }
}

/// Builds the fog uniform for this frame from the settings, the camera and the sun.
pub(crate) fn fog_uniform(s: &FogSettings, frame: &ChainFrame<'_>) -> FogUniform {
    let [near, far] = frame.near_far;
    let sun = unit_or_zero([
        frame.sun_direction[0],
        frame.sun_direction[1],
        frame.sun_direction[2],
    ]);
    let inv_view_proj = Mat4::from_cols_array_2d(&frame.camera.inv_view)
        * Mat4::from_cols_array_2d(&frame.camera.inv_proj);
    FogUniform {
        color_density: [s.color[0], s.color[1], s.color[2], s.density],
        falloff: [
            s.height_falloff,
            s.base_height,
            s.start_distance,
            s.max_opacity,
        ],
        scatter: [s.sun_scatter, s.anisotropy, near, far],
        camera_pos: frame.camera.camera_pos,
        sun_dir: [sun[0], sun[1], sun[2], 0.0],
        sun_color: [
            frame.sun_color[0],
            frame.sun_color[1],
            frame.sun_color[2],
            0.0,
        ],
        inv_view_proj: inv_view_proj.to_cols_array_2d(),
    }
}

/// Shader source of the fog pass.
pub(crate) const FOG_WGSL: &str = include_str!("../../shaders/fog.wgsl");
