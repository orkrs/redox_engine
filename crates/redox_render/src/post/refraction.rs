//! Refraction math (MAT-06 step 4): screen-space UV offset and the roughness
//! mip selection for sampling the scene-colour copy. Pure functions, no GPU
//! access. The GPU path (scene copy and shader branch) is not wired yet.

/// Smallest distance used for the offset, avoids huge offsets next to the camera.
const MIN_DISTANCE: f32 = 1.0;

/// Screen-space UV offset of a refracted lookup.
///
/// `normal_xy` is the view-space normal's xy tilt, `ior` the index of refraction,
/// `strength` an artist scale and `dist` the distance to the refracting surface.
/// The offset is `normal_xy * (ior - 1) * strength / max(dist, 1)`. It is zero
/// at `ior = 1`, grows with the normal tilt and shrinks with distance.
pub fn refraction_offset(ior: f32, normal_xy: [f32; 2], strength: f32, dist: f32) -> [f32; 2] {
    let k = (ior - 1.0) * strength / dist.max(MIN_DISTANCE);
    if !k.is_finite() {
        return [0.0, 0.0];
    }
    [normal_xy[0] * k, normal_xy[1] * k]
}

/// Mip level for a rough refraction lookup into a mip chain of `mip_count` levels.
///
/// The level scales linearly with `roughness` (clamped to 0..1) and is clamped
/// to `0..=mip_count - 1`. An empty chain gives 0.
pub fn refraction_mip_level(roughness: f32, mip_count: u32) -> f32 {
    if mip_count == 0 {
        return 0.0;
    }
    let max_level = (mip_count - 1) as f32;
    let r = if roughness.is_finite() {
        roughness.clamp(0.0, 1.0)
    } else {
        0.0
    };
    (r * max_level).clamp(0.0, max_level)
}
