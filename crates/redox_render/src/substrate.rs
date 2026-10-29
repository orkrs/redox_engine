//! Substrate-lite material model (MAT-04): energy-aware slabs and a clearcoat.
//!
//! This module is the CPU reference for the formulas that `shaders/pbr.wgsl`
//! mirrors (`coat_*`, `d_ggx_aniso`, `transmitted`). It has no GPU dependencies.
//!
//! Scope (v1): one base slab plus an optional clearcoat, a horizontal
//! parameter blend, and an anisotropic GGX distribution. Three or more layers
//! and thin film are out of scope.

use std::f32::consts::PI;

use redox_math::Vec3;

use crate::material::Material;

/// Lower bound of roughness used by the shading code (matches the WGSL clamp).
const MIN_ROUGHNESS: f32 = 0.05;
/// Largest |anisotropy| accepted; keeps both axis roughnesses above zero.
const MAX_ANISOTROPY: f32 = 0.99;
/// Default dielectric normal-incidence reflectance.
const DEFAULT_F0: f32 = 0.04;

/// Base layer of the Substrate stack: a single GGX lobe over a diffuse base.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slab {
    /// Base colour (linear RGB, expected in 0..1).
    pub base_color: [f32; 3],
    /// Metallic factor, 0..1.
    pub metallic: f32,
    /// Perceptual roughness, 0..1.
    pub roughness: f32,
    /// Anisotropy of the specular lobe, -1..1 (0 = isotropic).
    pub anisotropy: f32,
    /// Normal-incidence reflectance of the dielectric part (0.04 by default).
    pub f0: f32,
}

impl Default for Slab {
    fn default() -> Self {
        Self {
            base_color: [1.0; 3],
            metallic: 0.0,
            roughness: 0.5,
            anisotropy: 0.0,
            f0: DEFAULT_F0,
        }
    }
}

/// Clear coat on top of a slab. `weight == 0` means no coat.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coat {
    /// Coat strength, 0..1.
    pub weight: f32,
    /// Perceptual roughness of the coat lobe, 0..1.
    pub roughness: f32,
    /// Index of refraction of the coat (1.5 by default).
    pub ior: f32,
    /// Transmittance tint applied to light passing through the coat.
    pub tint: [f32; 3],
}

impl Default for Coat {
    fn default() -> Self {
        Self {
            weight: 0.0,
            roughness: 0.05,
            ior: 1.5,
            tint: [1.0; 3],
        }
    }
}

/// Normal-incidence reflectance of an interface with index `ior` against air.
pub fn f0_from_ior(ior: f32) -> f32 {
    let denom = (ior + 1.0).max(1.0e-4);
    let r = (ior - 1.0) / denom;
    (r * r).clamp(0.0, 1.0)
}

/// Schlick Fresnel with normal-incidence reflectance `f0` at cosine `cos_theta`.
fn schlick(f0: f32, cos_theta: f32) -> f32 {
    let c = cos_theta.clamp(0.0, 1.0);
    f0 + (1.0 - f0) * (1.0 - c).powi(5)
}

/// Fresnel reflectance (Schlick) of the coat at cosine `n_dot_v`.
pub fn coat_fresnel(coat: &Coat, n_dot_v: f32) -> f32 {
    schlick(f0_from_ior(coat.ior), n_dot_v)
}

/// Energy left for the layer below after the coat: `1 - weight * coat_fresnel`
/// times the tint transmittance. Per colour channel.
pub fn transmitted_through_coat(coat: &Coat, n_dot_v: f32) -> [f32; 3] {
    let w = coat.weight.clamp(0.0, 1.0);
    let k = 1.0 - w * coat_fresnel(coat, n_dot_v);
    coat.tint.map(|t| k * t.clamp(0.0, 1.0))
}

fn clamp_roughness(roughness: f32) -> f32 {
    roughness.clamp(MIN_ROUGHNESS, 1.0)
}

/// Isotropic GGX normal distribution. `roughness` is perceptual (alpha = roughness²).
pub fn d_ggx(n_dot_h: f32, roughness: f32) -> f32 {
    let a = clamp_roughness(roughness).powi(2);
    let a2 = a * a;
    let d = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    a2 / (PI * d * d)
}

/// Anisotropic GGX normal distribution (Burley 2012).
///
/// `h_dot_t` and `h_dot_b` are the half-vector components along the tangent
/// and bitangent. The axis roughnesses are `ax = a(1 + k)` and `ay = a(1 - k)`
/// with `a = roughness²` and `k = anisotropy`. At `k = 0` this equals [`d_ggx`]
/// for unit half-vectors.
pub fn d_ggx_aniso(
    n_dot_h: f32,
    h_dot_t: f32,
    h_dot_b: f32,
    roughness: f32,
    anisotropy: f32,
) -> f32 {
    let a = clamp_roughness(roughness).powi(2);
    let k = anisotropy.clamp(-MAX_ANISOTROPY, MAX_ANISOTROPY);
    let ax = a * (1.0 + k);
    let ay = a * (1.0 - k);
    let t = h_dot_t / ax;
    let b = h_dot_b / ay;
    let term = t * t + b * b + n_dot_h * n_dot_h;
    1.0 / (PI * ax * ay * term * term)
}

/// Smith G1 term for GGX at cosine `cos_theta` (separable form, in 0..1).
fn smith_g1(cos_theta: f32, roughness: f32) -> f32 {
    let alpha2 = clamp_roughness(roughness).powi(4);
    let c = cos_theta.clamp(1.0e-4, 1.0);
    2.0 * c / (c + (alpha2 + (1.0 - alpha2) * c * c).sqrt())
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let len_sq = dot3(v, v);
    if len_sq <= 1.0e-20 {
        return [0.0, 0.0, 1.0];
    }
    let inv = len_sq.sqrt().recip();
    [v[0] * inv, v[1] * inv, v[2] * inv]
}

fn mean3(v: [f32; 3]) -> f32 {
    (v[0] + v[1] + v[2]) / 3.0
}

/// Samples a visible normal (Heitz 2018) for view `v` in the local frame
/// where the surface normal is +z. `alpha` is roughness².
fn sample_visible_normal(v: [f32; 3], alpha: f32, u1: f32, u2: f32) -> [f32; 3] {
    let vh = normalize3([alpha * v[0], alpha * v[1], v[2]]);
    let len_sq = vh[0] * vh[0] + vh[1] * vh[1];
    let t1 = if len_sq > 1.0e-12 {
        let inv = len_sq.sqrt().recip();
        [-vh[1] * inv, vh[0] * inv, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let t2 = cross3(vh, t1);
    let r = u1.clamp(0.0, 1.0).sqrt();
    let phi = 2.0 * PI * u2;
    let p1 = r * phi.cos();
    let p2_raw = r * phi.sin();
    let s = 0.5 * (1.0 + vh[2]);
    let p2 = (1.0 - s) * (1.0 - p1 * p1).max(0.0).sqrt() + s * p2_raw;
    let p3 = (1.0 - p1 * p1 - p2 * p2).max(0.0).sqrt();
    let nh = [
        p1 * t1[0] + p2 * t2[0] + p3 * vh[0],
        p1 * t1[1] + p2 * t2[1] + p3 * vh[1],
        p1 * t1[2] + p2 * t2[2] + p3 * vh[2],
    ];
    normalize3([alpha * nh[0], alpha * nh[1], nh[2].max(0.0)])
}

/// Specular directional albedo of a GGX lobe with reflectance `f0`, and the
/// mean Fresnel over the same samples. Returns `(spec_albedo, mean_fresnel)`.
///
/// Uses a deterministic stratified grid over visible normals. Each sample
/// contributes `F * G1(l)`, which is at most `F`, so `spec_albedo <= mean_fresnel`
/// holds exactly for every sample count.
fn ggx_reflectance(f0: f32, roughness: f32, n_dot_v: f32, samples: u32) -> (f32, f32) {
    let nv = n_dot_v.clamp(1.0e-4, 1.0);
    let alpha = clamp_roughness(roughness).powi(2);
    let v = [(1.0 - nv * nv).max(0.0).sqrt(), 0.0, nv];
    let side = (samples.max(1) as f32).sqrt().ceil().max(1.0) as u32;
    let inv_side = 1.0 / side as f32;
    let mut spec = 0.0_f32;
    let mut fresnel = 0.0_f32;
    for i in 0..side {
        for j in 0..side {
            let u1 = (i as f32 + 0.5) * inv_side;
            let u2 = (j as f32 + 0.5) * inv_side;
            let h = sample_visible_normal(v, alpha, u1, u2);
            let vh = dot3(v, h).max(0.0);
            let l = [
                2.0 * vh * h[0] - v[0],
                2.0 * vh * h[1] - v[1],
                2.0 * vh * h[2] - v[2],
            ];
            if l[2] <= 0.0 {
                continue;
            }
            let f = schlick(f0, vh);
            spec += f * smith_g1(l[2], roughness);
            fresnel += f;
        }
    }
    let count = (side * side) as f32;
    (spec / count, fresnel / count)
}

/// Directional albedo of a slab at `n_dot_v`, in 0..1 for any parameters with
/// base colour components in 0..1.
///
/// Specular part: GGX with `f0 = mix(slab.f0, base, metallic)`. Diffuse part:
/// Lambert weighted by `(1 - mean Fresnel) * (1 - metallic)`. The bound holds
/// because the specular part never exceeds the mean Fresnel.
pub fn slab_albedo(slab: &Slab, n_dot_v: f32, samples: u32) -> f32 {
    let base = slab.base_color.map(|c| c.clamp(0.0, 1.0));
    let base_mean = mean3(base);
    let m = slab.metallic.clamp(0.0, 1.0);
    let f0 = slab.f0.clamp(0.0, 1.0) * (1.0 - m) + base_mean * m;
    let (spec, mean_f) = ggx_reflectance(f0, slab.roughness, n_dot_v, samples);
    spec + (1.0 - mean_f) * (1.0 - m) * base_mean
}

/// Total albedo of `Vertical(coat, slab)` at `n_dot_v`.
///
/// Coat reflection plus the slab albedo attenuated by the coat transmittance.
/// A zero-weight coat leaves the slab albedo unchanged.
pub fn vertical_albedo(coat: &Coat, slab: &Slab, n_dot_v: f32, samples: u32) -> f32 {
    let w = coat.weight.clamp(0.0, 1.0);
    if w <= 0.0 {
        return slab_albedo(slab, n_dot_v, samples);
    }
    let (coat_spec, _) = ggx_reflectance(f0_from_ior(coat.ior), coat.roughness, n_dot_v, samples);
    let transmit = mean3(transmitted_through_coat(coat, n_dot_v));
    w * coat_spec + transmit * slab_albedo(slab, n_dot_v, samples)
}

/// Linear blend of the parameters of two slabs (`Horizontal(a, b, mask)`, v1).
///
/// `mask = 0` gives `a`, `mask = 1` gives `b`. The blend mixes parameters, not
/// lighting results. Mixing results for strongly different metals is out of scope.
pub fn mix_slabs(a: &Slab, b: &Slab, mask: f32) -> Slab {
    let t = mask.clamp(0.0, 1.0);
    let mix = |x: f32, y: f32| x * (1.0 - t) + y * t;
    Slab {
        base_color: [
            mix(a.base_color[0], b.base_color[0]),
            mix(a.base_color[1], b.base_color[1]),
            mix(a.base_color[2], b.base_color[2]),
        ],
        metallic: mix(a.metallic, b.metallic),
        roughness: mix(a.roughness, b.roughness),
        anisotropy: mix(a.anisotropy, b.anisotropy),
        f0: mix(a.f0, b.f0),
    }
}

/// Builds the base slab that corresponds to a legacy [`Material`].
///
/// Legacy materials have no anisotropy and the default dielectric `f0`.
pub fn slab_from_material(material: &Material) -> Slab {
    let c: Vec3 = material.base_color;
    Slab {
        base_color: [c.x, c.y, c.z],
        metallic: material.metallic,
        roughness: material.roughness,
        anisotropy: material.anisotropy,
        f0: DEFAULT_F0,
    }
}

/// Returns the clearcoat of a legacy [`Material`], or `None` when it has none.
pub fn coat_from_material(material: &Material) -> Option<Coat> {
    if material.coat_weight <= 0.0 {
        return None;
    }
    let t: Vec3 = material.coat_tint;
    Some(Coat {
        weight: material.coat_weight,
        roughness: material.coat_roughness,
        ior: material.coat_ior,
        tint: [t.x, t.y, t.z],
    })
}
