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
                    screen_size_threshold: 0.02,
                },
                LodLevelConfig {
                    max_triangles: super::meshlet::MAX_TRIANGLES_PER_MESHLET,
                    triangle_fraction: 0.5,
                    screen_size_threshold: 0.005,
                },
                LodLevelConfig {
                    max_triangles: super::meshlet::MAX_TRIANGLES_PER_MESHLET,
                    triangle_fraction: 0.25,
                    screen_size_threshold: 0.0,
                },
            ],
        }
    }
}

/// Selects the best LOD level index for a given projected screen-space radius.
///
/// `projected_radius` should be in NDC space (0 = point, 1 = fills half screen).
/// Returns the LOD index (0 = most detail).
pub fn select_lod(projected_radius: f32, thresholds: &[f32]) -> usize {
    for (i, &threshold) in thresholds.iter().enumerate() {
        if projected_radius > threshold {
            return i;
        }
    }
    thresholds.len().saturating_sub(1)
}

/// Compute projected screen-space radius from world-space parameters.
///
/// - `sphere_radius`: world-space radius of the meshlet bounding sphere.
/// - `distance`:      distance from camera to sphere centre.
/// - `fov_y_rad`:     vertical field of view in radians.
pub fn projected_radius(sphere_radius: f32, distance: f32, fov_y_rad: f32) -> f32 {
    if distance < 1e-4 {
        return 1.0;
    }
    sphere_radius / (distance * (fov_y_rad * 0.5).tan())
}
