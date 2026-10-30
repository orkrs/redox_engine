//! Procedural noise (MAT-03 step 2): the CPU reference and the WGSL text of the same functions.
//!
//! The hash is an integer PCG, so no `sin` takes part and the CPU and the GPU see the same
//! lattice values. 2D and 3D share one code path: a 2D point has `z = 0`, and lattice corners
//! with `z = 1` are skipped, so a 2D result equals the 3D code restricted to the plane `z = 0`.
//!
//! Output ranges: value noise is in `[0, 1]`; gradient noise is roughly in `[-1, 1]`; Worley
//! returns the distance to the nearest feature point (F1).

/// Kind of a noise node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoiseKind {
    /// Lattice values blended with a smoothstep curve.
    Value,
    /// Lattice gradients (hash-derived, not normalised) blended like value noise.
    Gradient,
    /// Distance to the nearest jittered feature point of the 3x3(x3) neighbourhood.
    Worley,
}

impl NoiseKind {
    /// Name of the WGSL function of this kind.
    pub fn wgsl_fn(self) -> &'static str {
        match self {
            NoiseKind::Value => "mg_value_noise",
            NoiseKind::Gradient => "mg_gradient_noise",
            NoiseKind::Worley => "mg_worley",
        }
    }
}

/// Integer PCG hash (same constants and operations as the WGSL `mg_pcg`).
pub fn pcg(v: u32) -> u32 {
    let state = v.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277_803_737);
    (word >> 22) ^ word
}

/// Pseudo-random value in `[0, 1]` for a lattice cell and a channel. Uses 24 bits, so the
/// largest value is exactly 1.0 and float rounding cannot leave the range.
pub fn hash01(cell: [i32; 3], channel: u32) -> f32 {
    let h = pcg((cell[0] as u32) ^ pcg((cell[1] as u32) ^ pcg((cell[2] as u32) ^ pcg(channel))));
    (h >> 8) as f32 / 16_777_215.0
}

/// Lattice offset of corner `k` (bit 0 along x, bit 1 along y, bit 2 along z).
fn corner_offset(k: u32) -> [i32; 3] {
    [(k & 1) as i32, ((k >> 1) & 1) as i32, ((k >> 2) & 1) as i32]
}

/// Interpolation weight of corner `k`. Axes beyond `dims` contribute weight 1.
fn corner_weight(u: [f32; 3], k: u32, dims: u32) -> f32 {
    let mut w = 1.0;
    for axis in 0..3u32 {
        if axis == 2 && dims == 2 {
            continue;
        }
        let high = (k >> axis) & 1 == 1;
        let t = u[axis as usize];
        w *= if high { t } else { 1.0 - t };
    }
    w
}

/// Whether corner `k` is part of the lattice for `dims` (2D ignores the `z = 1` corners).
fn corner_used(k: u32, dims: u32) -> bool {
    !(dims == 2 && (k & 4) != 0)
}

/// Cell index, fractional part and smoothstep weights of `p`.
fn lattice(p: [f32; 3]) -> ([i32; 3], [f32; 3], [f32; 3]) {
    let base = [p[0].floor(), p[1].floor(), p[2].floor()];
    let f = [p[0] - base[0], p[1] - base[1], p[2] - base[2]];
    let u = [
        f[0] * f[0] * (3.0 - 2.0 * f[0]),
        f[1] * f[1] * (3.0 - 2.0 * f[1]),
        f[2] * f[2] * (3.0 - 2.0 * f[2]),
    ];
    let cell = [base[0] as i32, base[1] as i32, base[2] as i32];
    (cell, f, u)
}

/// Value noise in `[0, 1]`. `dims` is 2 or 3; for 2D only the `z = 0` plane is used.
pub fn value(p: [f32; 3], dims: u32) -> f32 {
    let (cell, _, u) = lattice(p);
    let mut sum = 0.0;
    for k in 0..8u32 {
        if !corner_used(k, dims) {
            continue;
        }
        let off = corner_offset(k);
        let c = [cell[0] + off[0], cell[1] + off[1], cell[2] + off[2]];
        sum += corner_weight(u, k, dims) * hash01(c, 0);
    }
    sum
}

/// Gradient noise: dot products of hash gradients with the offsets to the lattice corners.
pub fn gradient(p: [f32; 3], dims: u32) -> f32 {
    let (cell, f, u) = lattice(p);
    let mut sum = 0.0;
    for k in 0..8u32 {
        if !corner_used(k, dims) {
            continue;
        }
        let off = corner_offset(k);
        let c = [cell[0] + off[0], cell[1] + off[1], cell[2] + off[2]];
        let gz = if dims == 2 {
            0.0
        } else {
            hash01(c, 3) * 2.0 - 1.0
        };
        let g = [hash01(c, 1) * 2.0 - 1.0, hash01(c, 2) * 2.0 - 1.0, gz];
        let d = [
            f[0] - off[0] as f32,
            f[1] - off[1] as f32,
            if dims == 2 { 0.0 } else { f[2] - off[2] as f32 },
        ];
        let dot = g[0] * d[0] + g[1] * d[1] + g[2] * d[2];
        sum += corner_weight(u, k, dims) * dot;
    }
    sum
}

/// Worley F1: distance to the nearest jittered feature point among the neighbouring cells.
pub fn worley(p: [f32; 3], dims: u32) -> f32 {
    let base = [p[0].floor(), p[1].floor(), p[2].floor()];
    let cell = [base[0] as i32, base[1] as i32, base[2] as i32];
    let zs: &[i32] = if dims == 2 { &[0] } else { &[-1, 0, 1] };
    let mut best = f32::MAX;
    for &dz in zs {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let c = [cell[0] + dx, cell[1] + dy, cell[2] + dz];
                let feature = [
                    c[0] as f32 + hash01(c, 4),
                    c[1] as f32 + hash01(c, 5),
                    if dims == 2 {
                        0.0
                    } else {
                        c[2] as f32 + hash01(c, 6)
                    },
                ];
                let d = [feature[0] - p[0], feature[1] - p[1], feature[2] - p[2]];
                let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                best = best.min(dist);
            }
        }
    }
    best
}

/// Evaluates noise `kind` at `p` (only the first `dims` components are used).
pub fn eval(kind: NoiseKind, p: [f32; 3], dims: u32) -> f32 {
    let p = if dims == 2 { [p[0], p[1], 0.0] } else { p };
    match kind {
        NoiseKind::Value => value(p, dims),
        NoiseKind::Gradient => gradient(p, dims),
        NoiseKind::Worley => worley(p, dims),
    }
}

/// WGSL definitions of the hash and the three noise functions. Each takes `(p: vec3<f32>,
/// dims: u32)`; the code generator appends this text when a graph uses noise.
pub const WGSL: &str = r#"
fn mg_pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn mg_hash01(cell: vec3<i32>, channel: u32) -> f32 {
    let h = mg_pcg(bitcast<u32>(cell.x) ^ mg_pcg(bitcast<u32>(cell.y) ^ mg_pcg(bitcast<u32>(cell.z) ^ mg_pcg(channel))));
    return f32(h >> 8u) / 16777215.0;
}

fn mg_corner_weight(u: vec3<f32>, k: u32, dims: u32) -> f32 {
    let wx = select(1.0 - u.x, u.x, (k & 1u) != 0u);
    let wy = select(1.0 - u.y, u.y, (k & 2u) != 0u);
    let wz = select(select(1.0 - u.z, u.z, (k & 4u) != 0u), 1.0, dims == 2u);
    return wx * wy * wz;
}

fn mg_value_noise(p: vec3<f32>, dims: u32) -> f32 {
    let base = floor(p);
    let cell = vec3<i32>(base);
    let u = (p - base) * (p - base) * (3.0 - 2.0 * (p - base));
    var sum = 0.0;
    for (var k = 0u; k < 8u; k = k + 1u) {
        if (dims == 2u && (k & 4u) != 0u) {
            continue;
        }
        let off = vec3<i32>(i32(k & 1u), i32((k >> 1u) & 1u), i32((k >> 2u) & 1u));
        sum = sum + mg_corner_weight(u, k, dims) * mg_hash01(cell + off, 0u);
    }
    return sum;
}

fn mg_gradient_noise(p: vec3<f32>, dims: u32) -> f32 {
    let base = floor(p);
    let cell = vec3<i32>(base);
    let f = p - base;
    let u = f * f * (3.0 - 2.0 * f);
    var sum = 0.0;
    for (var k = 0u; k < 8u; k = k + 1u) {
        if (dims == 2u && (k & 4u) != 0u) {
            continue;
        }
        let off = vec3<i32>(i32(k & 1u), i32((k >> 1u) & 1u), i32((k >> 2u) & 1u));
        let c = cell + off;
        let gz = select(mg_hash01(c, 3u) * 2.0 - 1.0, 0.0, dims == 2u);
        let g = vec3<f32>(mg_hash01(c, 1u) * 2.0 - 1.0, mg_hash01(c, 2u) * 2.0 - 1.0, gz);
        let d = vec3<f32>(f.x - f32(off.x), f.y - f32(off.y), select(f.z - f32(off.z), 0.0, dims == 2u));
        sum = sum + mg_corner_weight(u, k, dims) * dot(g, d);
    }
    return sum;
}

fn mg_worley(p: vec3<f32>, dims: u32) -> f32 {
    let cell = vec3<i32>(floor(p));
    var best = 3.4e38;
    for (var dz = -1; dz <= 1; dz = dz + 1) {
        for (var dy = -1; dy <= 1; dy = dy + 1) {
            for (var dx = -1; dx <= 1; dx = dx + 1) {
                if (dims == 2u && dz != 0) {
                    continue;
                }
                let c = cell + vec3<i32>(dx, dy, dz);
                let feature = vec3<f32>(c) + vec3<f32>(mg_hash01(c, 4u), mg_hash01(c, 5u), select(mg_hash01(c, 6u), 0.0, dims == 2u));
                best = min(best, length(feature - p));
            }
        }
    }
    return best;
}
"#;
