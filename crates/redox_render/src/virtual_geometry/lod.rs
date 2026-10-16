//! LOD (Level of Detail) management for virtual geometry.
//!
//! Each LOD level is a set of meshlets that represent the same geometry at a
//! reduced triangle count.  LOD 0 is the highest quality; higher levels have
//! progressively more aggressive simplification.
//!
//! ## Runtime LOD selection
//!
//! The screen-space projected size of a meshlet's bounding sphere determines
//! which LOD level to use.  The GPU vertex shader (or a pre-pass compute
//! shader) computes:
//!
//! ```text
//! projected_radius = sphere_radius / (distance * tan(fov_y / 2))
//! if projected_radius > LOD_THRESHOLD_0 → use LOD 0
//! if projected_radius > LOD_THRESHOLD_1 → use LOD 1
//! ...
//! ```

/// Configuration for a single LOD level.
#[derive(Clone, Debug)]
pub struct LodLevelConfig {
    /// Maximum number of triangles per meshlet at this level.
    pub max_triangles: usize,
    /// Fraction of source triangles to retain (0.0–1.0).
    /// 1.0 = full detail (LOD 0), 0.5 = 50% reduction, etc.
    pub triangle_fraction: f32,
    /// Upper bound on the simplification error introduced by this level,
    /// relative to the mesh extent (quadric decimation stops early if it
    /// would exceed this). Only used during the asset build; 0 is treated as
    /// "unused" for full-detail levels.
    pub max_relative_error: f32,
    /// Minimum screen-space projected radius (normalised, 0–1) below which
    /// this LOD level is chosen.  Not used during asset build; only a hint
    /// for runtime selection.
    pub screen_size_threshold: f32,
}

impl Default for LodLevelConfig {
    fn default() -> Self {
        Self {
            max_triangles: super::meshlet::MAX_TRIANGLES_PER_MESHLET,
            triangle_fraction: 1.0,
            max_relative_error: 0.0,
            screen_size_threshold: 0.0,
        }
    }
}

/// Full LOD chain configuration passed to the asset pipeline.
#[derive(Clone, Debug)]
pub struct LodChainConfig {
    pub levels: Vec<LodLevelConfig>,
}

impl Default for LodChainConfig {
    /// Produces a 3-level LOD chain: full detail, 50%, and 25%.
    fn default() -> Self {
        Self {
            levels: vec![
                LodLevelConfig {
                    max_triangles: super::meshlet::MAX_TRIANGLES_PER_MESHLET,
                    triangle_fraction: 1.0,
                    max_relative_error: 0.0,
                    screen_size_threshold: 0.02,
                },
                LodLevelConfig {
                    max_triangles: super::meshlet::MAX_TRIANGLES_PER_MESHLET,
                    triangle_fraction: 0.5,
                    max_relative_error: 0.05,
                    screen_size_threshold: 0.005,
                },
                LodLevelConfig {
                    max_triangles: super::meshlet::MAX_TRIANGLES_PER_MESHLET,
                    triangle_fraction: 0.25,
                    max_relative_error: 0.15,
                    screen_size_threshold: 0.0,
                },
            ],
        }
    }
}

/// Selects the best LOD level index for a projected screen-space radius.
///
/// `projected_screen_radius` is the normalised projected radius produced by
/// [`projected_radius`] (NDC space: 0 = point, 1 = fills half the screen).
/// Levels are scanned finest → coarsest; the first level whose
/// `screen_size_threshold` is below the radius wins. A huge radius (object
/// close to the camera) therefore selects level 0 (finest), a radius of 0
/// selects the last level (coarsest).
///
/// This is a pure, hysteresis-free function. It is robust against bad input:
/// non-finite radii select level 0 (finest — safest visually) instead of
/// accidentally falling through failed comparisons, negative radii clamp to
/// zero, and non-finite thresholds never trigger. The qualifying level set
/// grows monotonically with the radius, so the result is monotone in
/// `projected_screen_radius` even for misordered thresholds.
pub fn select_lod(projected_screen_radius: f32, cfg: &LodChainConfig) -> usize {
    if cfg.levels.is_empty() {
        return 0;
    }
    if !projected_screen_radius.is_finite() {
        return 0;
    }
    let radius = projected_screen_radius.max(0.0);

    // First (finest) level whose screen-size threshold is met.
    for (i, level) in cfg.levels.iter().enumerate() {
        let t = level.screen_size_threshold;
        if t.is_finite() && radius > t {
            return i;
        }
    }
    cfg.levels.len() - 1
}

/// Compute projected screen-space radius from world-space parameters.
///
/// - `sphere_radius`: world-space radius of the meshlet bounding sphere.
/// - `distance`:      distance from camera to sphere centre.
/// - `fov_y_rad`:     vertical field of view in radians.
///
/// Returns at most 1.0 for objects at or behind the camera plane.
pub fn projected_radius(sphere_radius: f32, distance: f32, fov_y_rad: f32) -> f32 {
    if !distance.is_finite() || !sphere_radius.is_finite() || distance < 1e-4 {
        return 1.0;
    }
    let r = sphere_radius / (distance * (fov_y_rad * 0.5).tan());
    if r.is_finite() { r.max(0.0) } else { 1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_radius_selects_coarsest_level() {
        let cfg = LodChainConfig::default();
        assert_eq!(select_lod(0.0, &cfg), cfg.levels.len() - 1);
        assert_eq!(select_lod(-5.0, &cfg), cfg.levels.len() - 1);
    }

    #[test]
    fn huge_radius_selects_finest_level() {
        let cfg = LodChainConfig::default();
        assert_eq!(select_lod(100.0, &cfg), 0);
        assert_eq!(select_lod(f32::INFINITY, &cfg), 0);
        assert_eq!(select_lod(f32::NAN, &cfg), 0);
    }

    #[test]
    fn boundaries_and_midpoints() {
        // Default chain thresholds: [0.02, 0.005, 0.0].
        let cfg = LodChainConfig::default();
        // Just above threshold 0 → finest.
        assert_eq!(select_lod(0.02 + f32::EPSILON * 8.0, &cfg), 0);
        // Exactly on threshold 0 → next level.
        assert_eq!(select_lod(0.02, &cfg), 1);
        // Midpoint between thresholds 0 and 1.
        assert_eq!(select_lod((0.02 + 0.005) * 0.5, &cfg), 1);
        // Just above threshold 1 → middle level.
        assert_eq!(select_lod(0.005 + f32::EPSILON * 4.0, &cfg), 1);
        // Exactly on threshold 1 → coarsest.
        assert_eq!(select_lod(0.005, &cfg), 2);
    }

    #[test]
    fn unsorted_thresholds_still_monotone() {
        let cfg = LodChainConfig {
            levels: vec![
                super::super::LodLevelConfig {
                    max_triangles: 124,
                    triangle_fraction: 1.0,
                    max_relative_error: 0.0,
                    screen_size_threshold: 0.01,
                },
                super::super::LodLevelConfig {
                    max_triangles: 124,
                    triangle_fraction: 0.5,
                    max_relative_error: 0.05,
                    screen_size_threshold: 0.03,
                },
                super::super::LodLevelConfig {
                    max_triangles: 124,
                    triangle_fraction: 0.25,
                    max_relative_error: 0.15,
                    screen_size_threshold: 0.0,
                },
            ],
        };
        // Misordered middle threshold: level 0 qualifies first whenever its
        // (lower) threshold is met, so selection stays deterministic and
        // monotone in the radius — garbage in, best-effort out.
        assert_eq!(select_lod(0.04, &cfg), 0);
        assert_eq!(select_lod(0.02, &cfg), 0);
        assert_eq!(select_lod(0.001, &cfg), 2);
    }

    #[test]
    fn empty_chain_falls_back_to_finest() {
        let cfg = LodChainConfig { levels: vec![] };
        assert_eq!(select_lod(0.5, &cfg), 0);
    }

    #[test]
    fn projected_radius_math() {
        // fov 90° → tan(45°) = 1; radius 1 at distance 1 fills the screen.
        assert!((projected_radius(1.0, 1.0, std::f32::consts::FRAC_PI_2) - 1.0).abs() < 1e-6);
        // Known value: 2 / (4 · tan(30°)) ≈ 0.866.
        let r = projected_radius(2.0, 4.0, std::f32::consts::FRAC_PI_3);
        assert!((r - 0.866_025_4).abs() < 1e-6);
        // At / behind the camera clamps to full-screen.
        assert_eq!(projected_radius(1.0, 0.0, 1.0), 1.0);
        assert_eq!(projected_radius(1.0, -3.0, 1.0), 1.0);
    }
}
