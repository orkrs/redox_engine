//! CPU path-tracing reference for diffuse GI on the analytic box scene.
//!
//! Test and offline use only: it is the ground truth the probe pipeline is compared against.

use super::sdf_ref::{RefScene, sphere_trace};
use redox_math::Vec3;

/// Maximum trace distance; closed scenes always hit before this.
const REF_TMAX: f32 = 100.0;
/// Offset along the normal that keeps secondary rays off the surface they leave.
const RAY_OFFSET: f32 = 1e-3;
/// Largest supported number of surface bounces.
pub const MAX_BOUNCES: u32 = 4;

/// Deterministic PCG-style generator; no external crates.
#[derive(Clone, Copy, Debug)]
pub struct Pcg32 {
    state: u32,
}

impl Pcg32 {
    /// Generator seeded from `seed`.
    pub fn new(seed: u32) -> Self {
        Self {
            state: seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453),
        }
    }

    /// Next 32-bit value.
    pub fn next_u32(&mut self) -> u32 {
        let state = self
            .state
            .wrapping_mul(747_796_405)
            .wrapping_add(2_891_336_453);
        self.state = state;
        let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277_803_737);
        (word >> 22) ^ word
    }

    /// Next uniform float in `[0, 1)`.
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / 16_777_216.0)
    }
}

/// Cosine-weighted direction around the unit normal `n`.
fn cosine_direction(n: Vec3, rng: &mut Pcg32) -> Vec3 {
    let u1 = rng.next_f32();
    let u2 = rng.next_f32();
    let r = u1.sqrt();
    let phi = std::f32::consts::TAU * u2;
    let local = Vec3::new(r * phi.cos(), r * phi.sin(), (1.0 - u1).max(0.0).sqrt());
    let helper = if n.x.abs() > 0.9 { Vec3::Y } else { Vec3::X };
    let tangent = helper.cross(n).normalize_or_zero();
    let bitangent = n.cross(tangent);
    tangent * local.x + bitangent * local.y + n * local.z
}

/// Diffuse outgoing radiance of a white Lambertian surface at `pos` facing `normal`.
///
/// Estimated with `samples` cosine-weighted paths of at most `bounces` surface hits
/// (clamped to [`MAX_BOUNCES`]). For albedo 1 this equals the irradiance divided by pi,
/// the quantity a probe stores before multiplying by albedo.
pub fn reference_irradiance(
    scene: &RefScene,
    pos: Vec3,
    normal: Vec3,
    bounces: u32,
    samples: u32,
    seed: u32,
) -> Vec3 {
    let bounces = bounces.min(MAX_BOUNCES);
    let normal = normal.normalize_or_zero();
    let mut rng = Pcg32::new(seed);
    let mut sum = Vec3::ZERO;
    for _ in 0..samples {
        let dir = cosine_direction(normal, &mut rng);
        let origin = pos + normal * RAY_OFFSET;
        sum += trace_path(scene, origin, dir, bounces, &mut rng);
    }
    sum / samples.max(1) as f32
}

/// Follows one path for `bounces` surface hits and returns the emitted radiance it collects.
fn trace_path(
    scene: &RefScene,
    mut origin: Vec3,
    mut dir: Vec3,
    bounces: u32,
    rng: &mut Pcg32,
) -> Vec3 {
    let mut throughput = Vec3::ONE;
    let mut radiance = Vec3::ZERO;
    for _ in 0..bounces {
        let Some(hit) = sphere_trace(scene, origin, dir, REF_TMAX) else {
            break;
        };
        let surface = &scene.boxes[hit.index];
        let point = origin + dir * hit.t;
        radiance += throughput * surface.emissive;
        throughput *= surface.albedo;
        origin = point + hit.normal * RAY_OFFSET;
        dir = cosine_direction(hit.normal, rng);
    }
    radiance
}
