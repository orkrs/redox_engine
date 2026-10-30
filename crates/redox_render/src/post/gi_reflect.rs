//! Rough reflections from the GI cache (GI-04): settings and the pure functions mirrored in
//! `shaders/gi_reflect.wgsl`. The cache is the global SDF with the probe irradiance of the previous
//! frame (hit radiance = emission + albedo * E / pi); a screen-space pass in the post chain blends
//! the result over the colour before SSR.
//!
//! Known simplification: hit points are shaded from the probes, not from the surface cards, so the
//! reflection is as soft as the probe field and needs no card LOD.

/// Shader source of the blend pass.
pub(crate) const GI_REFLECT_APPLY_WGSL: &str = include_str!("../../shaders/gi_reflect_apply.wgsl");

/// Largest blend weight, so a cache reflection never fully replaces the surface colour.
const MAX_WEIGHT: f32 = 0.9;

/// Parameters of the cache reflections.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GiReflectSettings {
    /// Smoother surfaces are left to SSR and the probes. Default 0.05.
    pub min_roughness: f32,
    /// Rougher surfaces get no cache reflection. Default 0.7.
    pub max_roughness: f32,
    /// Longest reflected ray, metres. Default 30.
    pub max_distance: f32,
    /// Overall strength 0..1. Default 1.
    pub intensity: f32,
}

impl Default for GiReflectSettings {
    fn default() -> Self {
        Self {
            min_roughness: 0.05,
            max_roughness: 0.7,
            max_distance: 30.0,
            intensity: 1.0,
        }
    }
}

/// Cone half-angle (radians) for a perceptual roughness: `atan(roughness^2 * 0.5)` in `[0.01, 0.6]`.
pub fn cone_half_angle(roughness: f32) -> f32 {
    (roughness * roughness * 0.5).atan().clamp(0.01, 0.6)
}

fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    if hi <= lo {
        return if x >= hi { 1.0 } else { 0.0 };
    }
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// 0 outside `[min, max]`, a smooth ramp over the outer 20 % of the range at both ends.
pub fn roughness_range(roughness: f32, s: &GiReflectSettings) -> f32 {
    let edge = 0.2 * (s.max_roughness - s.min_roughness);
    smoothstep(s.min_roughness, s.min_roughness + edge, roughness)
        * (1.0 - smoothstep(s.max_roughness - edge, s.max_roughness, roughness))
}

/// Blend weight: `roughness_range * fresnel(n_dot_v, metallic) * intensity`, clamped to `0..=0.9`.
pub fn cache_weight(roughness: f32, metallic: f32, n_dot_v: f32, s: &GiReflectSettings) -> f32 {
    let f0 = 0.04 + 0.96 * metallic.clamp(0.0, 1.0);
    let fresnel = f0 + (1.0 - f0) * (1.0 - n_dot_v.clamp(0.0, 1.0)).powi(5);
    let w = roughness_range(roughness, s) * fresnel * s.intensity;
    if w.is_finite() {
        w.clamp(0.0, MAX_WEIGHT)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rougher_surface_has_a_wider_cone() {
        let mut last = 0.0;
        for i in 1..=10 {
            let a = cone_half_angle(i as f32 / 10.0);
            assert!(a >= last);
            last = a;
        }
    }

    #[test]
    fn the_cone_is_clamped_at_both_ends() {
        assert_eq!(cone_half_angle(0.0), 0.01);
        assert!(cone_half_angle(1.0) <= 0.6);
    }

    #[test]
    fn the_weight_is_zero_for_mirror_and_for_very_rough_surfaces() {
        let s = GiReflectSettings::default();
        assert_eq!(cache_weight(0.0, 1.0, 1.0, &s), 0.0);
        assert_eq!(cache_weight(0.9, 1.0, 1.0, &s), 0.0);
        assert!(cache_weight(0.4, 1.0, 1.0, &s) > 0.5);
    }

    #[test]
    fn a_metal_reflects_more_than_a_dielectric_at_normal_incidence() {
        let s = GiReflectSettings::default();
        assert!(cache_weight(0.4, 1.0, 1.0, &s) > cache_weight(0.4, 0.0, 1.0, &s));
    }

    #[test]
    fn the_weight_grows_towards_grazing_angles() {
        let s = GiReflectSettings::default();
        assert!(cache_weight(0.4, 0.0, 0.1, &s) > cache_weight(0.4, 0.0, 1.0, &s));
    }

    #[test]
    fn the_range_ramps_smoothly_at_both_ends() {
        let s = GiReflectSettings::default();
        let mut previous = 0.0;
        for i in 0..=20 {
            let r = s.min_roughness + (s.max_roughness - s.min_roughness) * 0.2 * i as f32 / 20.0;
            let w = roughness_range(r, &s);
            assert!(w >= previous - 1e-6);
            previous = w;
        }
        assert!((roughness_range(0.4, &s) - 1.0).abs() < 1e-6);
    }
}
