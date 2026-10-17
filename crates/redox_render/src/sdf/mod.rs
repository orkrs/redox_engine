//! SDF foundation: distance-field baking, sparse brick registry, and the
//! CPU cone-tracing reference used by SDF shadows (and later by DDGI in
//! Phase 5).
//!
//! Layout:
//! * [`bake`] — CPU baker producing a 64³ signed distance field per mesh
//!   (`DistanceField64`, [`bake_mesh`]).
//! * [`atlas`] — CPU-side registry mapping mesh ids to placed bricks
//!   ([`SdfBrickAtlas`]).
//! * [`gpu_atlas`] — 128³ GPU atlas + multi-brick page table uploaded for PBR.
//! * [`SdfShadowScene`] — world-space queries over the atlas plus
//!   [`SdfShadowScene::trace_cone`], the ground-truth implementation of the
//!   cone-traced shadow integral. `docs/reference/shaders/sdf_shadow.wgsl` mirrors it
//!   one-to-one for the GPU path.
//!
//! # Cone integral (shared CPU/GPU contract)
//!
//! `trace_cone` sphere-marches the cone axis and accumulates occlusion with
//! the standard SDF-AO / voxel-cone-tracing integral: at distance `t` the
//! cone has radius `r = tan(aperture/2) · t`; the fraction of the cone
//! cross-section blocked by solid at that ring is `saturate(1 - sd/r)`,
//! weighted by the ring's angular coverage `min(1, dt/r)` and combined
//! **multiplicatively** (`vis *= 1 - α`) so overlapping rings do not
//! over-darken. The function returns *transmittance* in `[0, 1]`
//! (1 = fully lit, 0 = fully occluded) — the same quantity
//! `sdf_trace_shadow` in the WGSL module returns.
//!
//! `aperture` is the full cone angle in radians; `aperture → 0` degenerates
//! to a hard binary shadow ray.

pub mod atlas;
pub mod bake;
pub mod gpu_atlas;

pub use atlas::{BrickRef, SdfBrick, SdfBrickAtlas};
pub use bake::{DistanceField64, bake_mesh};
pub use gpu_atlas::{GPU_ATLAS_EXTENT, MAX_SDF_BRICKS, SdfBrickPageGpu, SdfGpuAtlas, SdfSceneGpu};

use redox_math::Vec3;

/// Hard cap on sphere-marching steps per cone trace.
const MAX_CONE_STEPS: u32 = 128;
/// Occlusion saturation threshold — stop marching once nothing is visible.
const VIS_EPS: f32 = 1.0e-3;

/// Scene-level SDF shadow queries over a brick atlas.
///
/// This is the CPU ground truth for `docs/reference/shaders/sdf_shadow.wgsl` and doubles as
/// the offline validation path (unit tests, future DDGI ray budget checks).
pub struct SdfShadowScene {
    /// The brick atlas.
    pub atlas: SdfBrickAtlas,
}

impl SdfShadowScene {
    /// Creates an empty scene.
    pub fn new() -> Self {
        Self {
            atlas: SdfBrickAtlas::new(),
        }
    }

    /// Smallest texel edge across all bricks (meters); drives the march
    /// step floor. `f32::MAX` for an empty scene (callers return early).
    fn min_texel_size(&self) -> f32 {
        self.atlas
            .iter()
            .map(|b| b.field.texel_world_size().min_element())
            .fold(f32::MAX, f32::min)
    }

    /// Signed distance at world-space `p`, trilinearly sampled from the
    /// brick whose domain contains `p` (the minimum over all containing
    /// bricks when domains overlap). Positions outside every brick report
    /// `f32::MAX` — "far outside any solid" — which the cone integral
    /// treats as unoccluded.
    pub fn sample_world(&self, p: Vec3) -> f32 {
        let mut sd = f32::MAX;
        for brick in self.atlas.iter() {
            if brick.contains(p) {
                sd = sd.min(brick.sample_world(p));
            }
        }
        sd
    }

    /// Cone-traced visibility from `origin` along `dir` (need not be
    /// normalised), returning transmittance in `[0, 1]`.
    ///
    /// See the module docs for the integral. Callers should offset `origin`
    /// off the surface along its normal first (the WGSL entry point does
    /// this internally via its `normal` parameter) to avoid self-shadowing.
    pub fn trace_cone(&self, origin: Vec3, dir: Vec3, aperture: f32, max_dist: f32) -> f32 {
        let dir = dir.normalize_or_zero();
        if dir == Vec3::ZERO || self.atlas.is_empty() {
            return 1.0;
        }
        let t_min = 0.5 * self.min_texel_size();
        let half_tan = (0.5 * aperture).tan();

        let mut visibility = 1.0f32;
        let mut t = t_min;
        for _ in 0..MAX_CONE_STEPS {
            if t >= max_dist {
                break;
            }
            let p = origin + dir * t;
            let sd = self.sample_world(p);
            let r = half_tan * t;

            // Fraction of the cone cross-section blocked at this ring.
            let occ = if r > 1.0e-6 {
                ((r - sd) / r).clamp(0.0, 1.0)
            } else if sd <= 0.0 {
                1.0
            } else {
                0.0
            };

            // Ring-consistent step: fine inside solids, otherwise at most
            // one cone radius per step (never below half a texel).
            let dt = if sd <= 0.0 {
                t_min
            } else {
                sd.abs().clamp(t_min, r.max(t_min))
            };
            // Angular coverage of this ring relative to the whole cone.
            let cov = (dt / r.max(t_min)).min(1.0);

            visibility *= 1.0 - occ * cov;
            if visibility < VIS_EPS {
                break;
            }
            t += dt;
        }
        visibility.clamp(0.0, 1.0)
    }
}

impl Default for SdfShadowScene {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::primitive::create_cube;

    /// Builds a scene holding one brick: unit cube baked over a 2 m domain
    /// centred at the world origin (cube surface at ±0.5 m).
    fn cube_scene() -> SdfShadowScene {
        let cube = create_cube();
        let field = bake_mesh(&cube, Vec3::splat(2.0));
        let mut scene = SdfShadowScene::new();
        scene.atlas.insert(
            1,
            SdfBrick {
                field,
                origin_ws: Vec3::ZERO,
                world_size: Vec3::splat(2.0),
            },
        );
        scene
    }

    #[test]
    fn cone_trace_visibility() {
        let scene = cube_scene();

        // Origin inside the solid: the very first ring is fully occluded.
        let inside = scene.trace_cone(Vec3::ZERO, Vec3::X, 0.3, 10.0);
        assert!(
            inside < 0.02,
            "cone from inside the solid must be fully occluded, got {inside}"
        );

        // Cone pointing away from the cube: nothing blocks it.
        let away = scene.trace_cone(Vec3::new(2.0, 0.9, 0.0), Vec3::X, 0.3, 10.0);
        assert!(
            away > 0.999,
            "unoccluded cone must transmit fully, got {away}"
        );

        // Grazing cone: axis passes 0.25 m above the top face; the cone
        // radius (tan(0.15)·t) overtakes that clearance only between
        // t ≈ 1.65 and t ≈ 2.8 — partway along the march → partial shadow.
        let grazing = scene.trace_cone(
            Vec3::new(2.0, 0.75, 0.0),
            Vec3::new(-1.0, 0.0, 0.0),
            0.3,
            4.0,
        );
        assert!(
            grazing > 0.1 && grazing < 0.9,
            "grazing cone must be partially occluded, got {grazing}"
        );

        // Directly at the cube face from outside: heavily (but not by
        // definition fully) occluded — must be strictly below full light.
        let at_face = scene.trace_cone(
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(-1.0, 0.0, 0.0),
            0.3,
            4.0,
        );
        assert!(
            at_face < 0.5,
            "cone aimed at the cube face should be mostly occluded, got {at_face}"
        );

        // Degenerate aperture (hard ray) at the face: fully blocked.
        let hard = scene.trace_cone(
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(-1.0, 0.0, 0.0),
            1.0e-6,
            4.0,
        );
        assert!(
            hard < 0.02,
            "hard ray at the face must be blocked, got {hard}"
        );

        // Empty scene: everything lit.
        assert_eq!(
            SdfShadowScene::new().trace_cone(Vec3::ZERO, Vec3::X, 0.3, 5.0),
            1.0
        );
    }

    #[test]
    fn sample_world_matches_field() {
        let scene = cube_scene();
        // 0.25 m outside the +X face → +0.25.
        let sd = scene.sample_world(Vec3::new(0.75, 0.0, 0.0));
        assert!((sd - 0.25).abs() < 1.0e-3, "got {sd}");
        // 0.25 m inside the +X face → -0.25 (trilinear across interior texels).
        let sd = scene.sample_world(Vec3::new(0.25, 0.0, 0.0));
        assert!((sd + 0.25).abs() < 1.0e-3, "got {sd}");
        // Outside the brick domain → "far outside".
        assert_eq!(scene.sample_world(Vec3::new(50.0, 0.0, 0.0)), f32::MAX);
    }
}
