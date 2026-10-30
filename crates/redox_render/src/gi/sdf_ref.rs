//! Analytic box scene and sphere tracing: the CPU reference for the global SDF.
//!
//! The GPU trace in `shaders/gi_trace.wgsl` marches the sampled volume; these functions
//! march the exact boxes, so tests can compare both against known geometry.

use crate::sdf::global::{SdfBox, sdf_aabb};
use redox_math::Vec3;

/// Distance under which a point counts as on the surface, in metres.
pub const SURFACE_EPS: f32 = 1e-4;
/// Upper bound on sphere-tracing iterations per ray.
const MAX_STEPS: u32 = 512;
/// Finite-difference step for the surface normal, in metres.
const NORMAL_EPS: f32 = 1e-3;

/// Set of boxes forming a static scene.
#[derive(Clone, Debug, Default)]
pub struct RefScene {
    /// The boxes; the scene distance is the minimum over them.
    pub boxes: Vec<SdfBox>,
}

/// First surface hit along a ray.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// Distance along the ray in metres.
    pub t: f32,
    /// Index of the box that was hit.
    pub index: usize,
    /// Unit surface normal at the hit point.
    pub normal: Vec3,
}

/// Signed distance from `p` to the box `b` (negative inside, exact outside).
pub fn sdf_box(p: Vec3, b: &SdfBox) -> f32 {
    sdf_aabb(p, b.min, b.max)
}

/// Minimum signed distance from `p` to the scene and the index of that box.
/// An empty scene returns `(f32::MAX, 0)`.
pub fn sdf_scene(scene: &RefScene, p: Vec3) -> (f32, usize) {
    let mut best = (f32::MAX, 0usize);
    for (i, b) in scene.boxes.iter().enumerate() {
        let d = sdf_box(p, b);
        if d < best.0 {
            best = (d, i);
        }
    }
    best
}

/// Sphere-traces the ray `origin + t * dir` up to `tmax`; `None` when nothing is hit.
pub fn sphere_trace(scene: &RefScene, origin: Vec3, dir: Vec3, tmax: f32) -> Option<Hit> {
    let dir = dir.normalize_or_zero();
    if dir == Vec3::ZERO {
        return None;
    }
    let mut t = 0.0f32;
    for _ in 0..MAX_STEPS {
        if t > tmax {
            break;
        }
        let p = origin + dir * t;
        let (d, index) = sdf_scene(scene, p);
        if d < SURFACE_EPS {
            return Some(Hit {
                t,
                index,
                normal: normal_at(scene, p),
            });
        }
        t += d;
    }
    None
}

/// Central-difference gradient of the scene distance, normalised.
fn normal_at(scene: &RefScene, p: Vec3) -> Vec3 {
    let e = NORMAL_EPS;
    let dist = |q: Vec3| sdf_scene(scene, q).0;
    let g = Vec3::new(
        dist(p + Vec3::X * e) - dist(p - Vec3::X * e),
        dist(p + Vec3::Y * e) - dist(p - Vec3::Y * e),
        dist(p + Vec3::Z * e) - dist(p - Vec3::Z * e),
    );
    g.normalize_or_zero()
}

/// Cornell-style room: 4 × 4 × 4 m interior with inner faces at `x, z = ±2` and `y ∈ [0, 4]`,
/// walls 0.2 m thick. Left wall red, right wall green, others white. A 1 × 1 m white
/// 0.1 m thick ceiling panel emits `(12, 12, 12)`. The room is closed (front wall included) so that
/// every ray from inside hits a surface.
pub fn cornell_box() -> RefScene {
    let white = Vec3::splat(0.75);
    let red = Vec3::new(0.75, 0.1, 0.1);
    let green = Vec3::new(0.1, 0.75, 0.1);
    let slab = |min: [f32; 3], max: [f32; 3], albedo: Vec3| SdfBox {
        min: Vec3::from_array(min),
        max: Vec3::from_array(max),
        albedo,
        emissive: Vec3::ZERO,
    };
    RefScene {
        boxes: vec![
            slab([-2.2, -0.2, -2.2], [2.2, 0.0, 2.2], white),
            slab([-2.2, 4.0, -2.2], [2.2, 4.2, 2.2], white),
            slab([-2.2, 0.0, -2.2], [2.2, 4.0, -2.0], white),
            // ASSUMPTION: the spec lists no front wall, but the escape test needs a closed room.
            slab([-2.2, 0.0, 2.0], [2.2, 4.0, 2.2], white),
            slab([-2.2, 0.0, -2.0], [-2.0, 4.0, 2.0], red),
            slab([2.0, 0.0, -2.0], [2.2, 4.0, 2.0], green),
            SdfBox {
                min: Vec3::new(-0.5, 3.9, -0.5),
                max: Vec3::new(0.5, 4.0, 0.5),
                albedo: Vec3::ONE,
                emissive: Vec3::splat(12.0),
            },
        ],
    }
}
