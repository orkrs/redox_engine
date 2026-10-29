//! Subsurface-scattering math (MAT-05): wrap lighting and the separable
//! screen-space kernel. CPU reference for `pbr.wgsl` (`wrap_diffuse`) and for
//! the planned `shaders/sss_blur.wgsl`. Pure functions, no GPU access.

/// One sample of the separable kernel: offset in kernel units (-1..1) and an RGB weight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KernelSample {
    /// Position along the blur direction, -1..1 (non-linear, dense near 0).
    pub offset: f32,
    /// Per-channel weight. Each channel sums to 1 over the kernel.
    pub weight: [f32; 3],
}

/// Smallest falloff width, keeps the Gaussian finite for a zero profile.
const MIN_FALLOFF: f32 = 1.0e-3;

/// Wrapped Lambert term: `max((n·l + w) / (1 + w), 0)` with `w` clamped to 0..1.
///
/// `wrap = 0` gives the plain Lambert term `max(n·l, 0)`, and `n·l = 1` maps to 1.
pub fn wrap_diffuse(n_dot_l: f32, wrap: f32) -> f32 {
    let w = wrap.clamp(0.0, 1.0);
    ((n_dot_l + w) / (1.0 + w)).max(0.0)
}

/// Builds an odd-sized separable kernel for the scattering colour `profile`.
///
/// Each channel is a Gaussian whose width is proportional to `profile[c]`, so
/// red (wide) scatters further than blue (narrow). Offsets follow `x * |x|`
/// on a uniform grid, which packs samples near the centre. Weights are
/// normalised per channel.
///
/// `taps` is rounded up to the next odd number, with a minimum of 1.
pub fn sss_kernel(profile: [f32; 3], taps: usize) -> Vec<KernelSample> {
    let n = taps.max(1) | 1;
    let half = (n - 1) as f32 / 2.0;
    let mut samples: Vec<KernelSample> = (0..n)
        .map(|i| {
            let x = if half > 0.0 {
                (i as f32 - half) / half
            } else {
                0.0
            };
            KernelSample {
                offset: x * x.abs(),
                weight: [0.0; 3],
            }
        })
        .collect();

    for (c, &channel) in profile.iter().enumerate() {
        let sigma = channel.max(MIN_FALLOFF);
        let inv_two_sigma_sq = 0.5 / (sigma * sigma);
        let mut sum = 0.0_f32;
        for s in samples.iter_mut() {
            let w = (-(s.offset * s.offset) * inv_two_sigma_sq).exp();
            s.weight[c] = w;
            sum += w;
        }
        // The centre sample always contributes 1, so `sum >= 1`.
        for s in samples.iter_mut() {
            s.weight[c] /= sum;
        }
    }
    samples
}

/// Blur width in pixels for a scattering radius `radius_mm` seen at `dist_m`.
///
/// Uses a screen of `screen_h` pixels with vertical field of view `fov_y`
/// (radians). Degenerate input (non-positive distance or FOV, negative radius)
/// gives 0, and the result is always finite.
pub fn blur_radius_pixels(radius_mm: f32, dist_m: f32, fov_y: f32, screen_h: f32) -> f32 {
    let half_tan = (fov_y * 0.5).tan();
    let valid = radius_mm > 0.0
        && dist_m > 1.0e-6
        && half_tan > 1.0e-6
        && screen_h > 0.0
        && radius_mm.is_finite()
        && dist_m.is_finite()
        && half_tan.is_finite();
    if !valid {
        return 0.0;
    }
    let radius_m = radius_mm * 1.0e-3;
    let visible_height_m = 2.0 * dist_m * half_tan;
    let px = radius_m * screen_h / visible_height_m;
    if px.is_finite() { px.max(0.0) } else { 0.0 }
}
