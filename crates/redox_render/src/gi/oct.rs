//! Octahedral direction mapping and sphere sampling shared by the GI CPU reference, the
//! probe atlas and the GPU trace (the WGSL mirror lives in `shaders/gi_trace.wgsl`).
//!
//! The unit sphere is unfolded onto the square `[0, 1]²`. `oct_decode(oct_encode(d))`
//! returns `d` for every unit direction `d`.

use redox_math::{Vec2, Vec3};

/// Golden ratio `(1 + sqrt(5)) / 2`.
const GOLDEN_RATIO: f32 = 1.618_034;

/// Maps a direction to octahedral coordinates in `[0, 1]²`. The input does not need to be
/// normalised; a zero vector maps to the centre of the square.
pub fn oct_encode(dir: Vec3) -> Vec2 {
    let n = dir / (dir.x.abs() + dir.y.abs() + dir.z.abs()).max(1e-6);
    let mut u = n.x;
    let mut v = n.z;
    if n.y < 0.0 {
        let ou = u;
        u = (1.0 - v.abs()) * sign_nonzero(ou);
        v = (1.0 - ou.abs()) * sign_nonzero(v);
    }
    Vec2::new(
        (u * 0.5 + 0.5).clamp(0.0, 1.0),
        (v * 0.5 + 0.5).clamp(0.0, 1.0),
    )
}

/// Inverse of [`oct_encode`]: octahedral coordinates in `[0, 1]²` to a unit direction.
pub fn oct_decode(uv: Vec2) -> Vec3 {
    let mut f = Vec3::new(uv.x * 2.0 - 1.0, 0.0, uv.y * 2.0 - 1.0);
    f.y = 1.0 - f.x.abs() - f.z.abs();
    if f.y < 0.0 {
        let ox = f.x;
        f.x = (1.0 - f.z.abs()) * sign_nonzero(ox);
        f.z = (1.0 - ox.abs()) * sign_nonzero(f.z);
    }
    f.normalize_or_zero()
}

/// The `index`-th of `count` near-uniform points on the unit sphere (Fibonacci lattice).
///
/// Used for the per-probe ray set; the GPU version `fib_dir` in `gi_trace.wgsl` must match it.
pub fn fibonacci_sphere(index: u32, count: u32) -> Vec3 {
    let i = index as f32 + 0.5;
    let z = 1.0 - 2.0 * i / count.max(1) as f32;
    let r = (1.0 - z * z).max(0.0).sqrt();
    let phi = i * golden_angle();
    Vec3::new(r * phi.cos(), r * phi.sin(), z)
}

/// Golden angle in radians, `TAU / phi²`.
pub fn golden_angle() -> f32 {
    std::f32::consts::TAU / (GOLDEN_RATIO * GOLDEN_RATIO)
}

/// Octahedral edge rule for the one-texel border of a probe tile with `inner` texels per side
/// (the WGSL mirror is `border_source` in `shaders/gi_update.wgsl`).
///
/// `texel` is a tile coordinate in `0..inner + 2`: the border ring sits at 0 and `inner + 1`.
/// The border texel's direction is decoded beyond the square, folded back by the octahedral
/// rule and re-encoded; the result is the inner texel (tile coordinates `1..=inner`) that
/// contains that direction. Inner texels map to themselves; corners map to the opposite
/// inner corner.
pub fn border_source(texel: [u32; 2], inner: u32) -> [u32; 2] {
    let n = inner as f32;
    let uv = Vec2::new((texel[0] as f32 - 0.5) / n, (texel[1] as f32 - 0.5) / n);
    let e = oct_encode(oct_decode(uv));
    let pick = |v: f32| ((v * n).floor().clamp(0.0, n - 1.0) as u32) + 1;
    [pick(e.x), pick(e.y)]
}

fn sign_nonzero(x: f32) -> f32 {
    if x >= 0.0 { 1.0 } else { -1.0 }
}
