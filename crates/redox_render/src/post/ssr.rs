//! Screen-space reflections: a linear march along the reflected ray through the frame's depth,
//! refined by bisection. Runs first in the post chain, on the TAA output.
//!
//! The pure functions here are mirrored in `shaders/ssr.wgsl`; keep both in sync.
//! Known simplification: the environment specular from PBR is already in the colour, so the
//! blend replaces part of it instead of adding to it (energy is lost, which is accepted).

use bytemuck::{Pod, Zeroable};

use crate::post::post_fx::ChainFrame;

/// Most steps the linear march may take (the uniform is clamped to this).
pub const MAX_SSR_STEPS: u32 = 128;
/// Upper bound of the blend weight, so a reflection never fully replaces the surface colour.
const MAX_WEIGHT: f32 = 0.9;
/// Edge fade band: the reflection fades to zero over this fraction of the screen at each border.
const EDGE_BAND: f32 = 0.1;

/// Parameters of the screen-space reflections.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SsrSettings {
    /// Reflections are drawn only on surfaces rougher-than this are skipped (0..1). Default 0.5.
    pub max_roughness: f32,
    /// Longest ray in view-space units. Default 30.0.
    pub max_distance: f32,
    /// Steps of the linear march (the hit is then refined by 5 bisection steps). Default 32.
    pub steps: u32,
    /// Depth thickness (view-space units) below which a ray behind a surface still counts as a hit. Default 0.3.
    pub thickness: f32,
    /// Overall strength 0..1. Default 1.0.
    pub intensity: f32,
}

impl Default for SsrSettings {
    fn default() -> Self {
        Self {
            max_roughness: 0.5,
            max_distance: 30.0,
            steps: 32,
            thickness: 0.3,
            intensity: 1.0,
        }
    }
}

/// 0 at `max_roughness` and above, 1 for a mirror; smooth in between: `(1 - r / max)^2` clamped.
///
/// A NaN input or a non-positive `max_roughness` gives 0.
pub fn roughness_weight(roughness: f32, max_roughness: f32) -> f32 {
    if !(roughness.is_finite() && max_roughness > 0.0) {
        return 0.0;
    }
    let t = (1.0 - roughness / max_roughness).clamp(0.0, 1.0);
    t * t
}

/// Fade towards the screen edges: 1 inside the central 80 %, 0 at the border, for uv in 0..1.
///
/// Each axis fades over the outer 10 % and the two factors multiply.
pub fn edge_fade(uv: [f32; 2]) -> f32 {
    let axis = |u: f32| (u.min(1.0 - u) / EDGE_BAND).clamp(0.0, 1.0);
    axis(uv[0]) * axis(uv[1])
}

/// Schlick Fresnel with F0 = mix(0.04, 1, metallic).
///
/// Grows towards grazing angles (`n_dot_v` near 0); metals stay strong at every angle.
pub fn fresnel_weight(n_dot_v: f32, metallic: f32) -> f32 {
    let f0 = 0.04 + (1.0 - 0.04) * metallic;
    let c = n_dot_v.clamp(0.0, 1.0);
    f0 + (1.0 - f0) * (1.0 - c).powi(5)
}

/// Final blend weight: `roughness_weight * edge_fade(hit_uv) * fresnel_weight * intensity`,
/// clamped to `0..=0.9`. A non-finite result gives 0.
pub fn ssr_weight(
    roughness: f32,
    metallic: f32,
    n_dot_v: f32,
    hit_uv: [f32; 2],
    s: &SsrSettings,
) -> f32 {
    let w = roughness_weight(roughness, s.max_roughness)
        * edge_fade(hit_uv)
        * fresnel_weight(n_dot_v, metallic)
        * s.intensity;
    if w.is_finite() {
        w.clamp(0.0, MAX_WEIGHT)
    } else {
        0.0
    }
}

/// Uniform of `shaders/ssr.wgsl`. 240 bytes: three matrices, then scalars padded to 16.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct SsrUniform {
    view: [[f32; 4]; 4],
    proj: [[f32; 4]; 4],
    inv_proj: [[f32; 4]; 4],
    screen: [f32; 2],
    max_roughness: f32,
    max_distance: f32,
    thickness: f32,
    intensity: f32,
    steps: u32,
    _pad: [u32; 5],
}

/// Builds the SSR uniform for this frame from the settings and the camera.
pub(crate) fn ssr_uniform(s: &SsrSettings, frame: &ChainFrame<'_>) -> SsrUniform {
    SsrUniform {
        view: frame.camera.view,
        proj: frame.camera.proj,
        inv_proj: frame.camera.inv_proj,
        screen: frame.screen,
        max_roughness: s.max_roughness,
        max_distance: s.max_distance,
        thickness: s.thickness,
        intensity: s.intensity,
        steps: s.steps.clamp(1, MAX_SSR_STEPS),
        _pad: [0; 5],
    }
}

/// Shader source of the reflection pass.
pub(crate) const SSR_WGSL: &str = include_str!("../../shaders/ssr.wgsl");
