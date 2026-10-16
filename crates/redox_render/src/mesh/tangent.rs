//! Tangent-frame generation for static geometry.
//!
//! Implements Eric Lengyel's classic method ("Computing Tangent Space Basis
//! Vectors for an Arbitrary Mesh"): tangent and bitangent contributions are
//! accumulated per triangle onto each vertex, then orthogonalised against
//! the vertex normal. The `w` component of the result encodes bitangent
//! handedness such that `cross(normal, tangent.xyz) * tangent.w` equals the
//! bitangent direction.
//!
//! This module is the canonical implementation for the render crate (used by
//! the OBJ loader); `redox_asset::loader::gltf_loader` carries a documented
//! local copy because the asset crate must not depend on the render crate.

/// Computes per-vertex tangents `[tx, ty, tz, handedness]`.
///
/// Returns exactly one entry per position. Triangles are taken from
/// `indices` (three consecutive indices per triangle); out-of-range indices
/// and degenerate UV determinants are skipped. Vertices whose accumulated
/// tangent is (near) zero — e.g. missing or constant UVs — fall back to
/// `[1, 0, 0, 1]`. The xyz part of every result is unit length.
pub fn compute_tangents(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &[u32],
) -> Vec<[f32; 4]> {
    /// Fallback tangent (+X, right-handed) for vertices without a usable frame.
    const FALLBACK: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

    let count = positions.len();
    let mut tan_acc = vec![[0.0f32; 3]; count];
    let mut bitan_acc = vec![[0.0f32; 3]; count];

    // Pass 1: accumulate triangle tangent/bitangent onto corner vertices.
    for tri in indices.chunks_exact(3) {
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        if i0 >= count || i1 >= count || i2 >= count {
            continue;
        }

        let p0 = positions[i0];
        let p1 = positions[i1];
        let p2 = positions[i2];
        let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];

        let (u0, v0) = lookup_uv(uvs, i0);
        let (u1, v1) = lookup_uv(uvs, i1);
        let (u2, v2) = lookup_uv(uvs, i2);
        let du1 = u1 - u0;
        let dv1 = v1 - v0;
        let du2 = u2 - u0;
        let dv2 = v2 - v0;

        let det = du1 * dv2 - du2 * dv1;
        if det.abs() <= f32::EPSILON {
            continue;
        }
        let r = 1.0 / det;

        // Tangent follows +U, bitangent follows +V in texture space.
        let t = [
            r * (dv2 * e1[0] - dv1 * e2[0]),
            r * (dv2 * e1[1] - dv1 * e2[1]),
            r * (dv2 * e1[2] - dv1 * e2[2]),
        ];
        let b = [
            r * (-du2 * e1[0] + du1 * e2[0]),
            r * (-du2 * e1[1] + du1 * e2[1]),
            r * (-du2 * e1[2] + du1 * e2[2]),
        ];

        for &i in &[i0, i1, i2] {
            tan_acc[i][0] += t[0];
            tan_acc[i][1] += t[1];
            tan_acc[i][2] += t[2];
            bitan_acc[i][0] += b[0];
            bitan_acc[i][1] += b[1];
            bitan_acc[i][2] += b[2];
        }
    }

    // Pass 2: Gram-Schmidt against the normal, then extract handedness.
    let mut result = Vec::with_capacity(count);
    for i in 0..count {
        let n = normal_or_up(normals, i);

        let t_unnorm = sub(tan_acc[i], scale(n, dot(n, tan_acc[i])));
        let t_len_sq = dot(t_unnorm, t_unnorm);
        if t_len_sq <= f32::EPSILON {
            result.push(FALLBACK);
            continue;
        }
        let t = scale(t_unnorm, 1.0 / t_len_sq.sqrt());

        let w = if dot(cross(n, t), bitan_acc[i]) >= 0.0 { 1.0 } else { -1.0 };
        result.push([t[0], t[1], t[2], w]);
    }
    result
}

/// Returns the UV pair for `index`, defaulting to the origin.
#[inline]
fn lookup_uv(uvs: &[[f32; 2]], index: usize) -> (f32, f32) {
    uvs.get(index).map(|uv| (uv[0], uv[1])).unwrap_or((0.0, 0.0))
}

/// Returns the normal for `index`, falling back to straight up.
#[inline]
fn normal_or_up(normals: &[[f32; 3]], index: usize) -> [f32; 3] {
    match normals.get(index).copied() {
        Some(n) => {
            let len_sq = dot(n, n);
            if len_sq > f32::EPSILON {
                scale(n, 1.0 / len_sq.sqrt())
            } else {
                [0.0, 1.0, 0.0]
            }
        }
        None => [0.0, 1.0, 0.0],
    }
}

#[inline]
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[inline]
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

#[cfg(test)]
mod tests {
    use super::*;

    const POSITIONS: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    const NORMALS_Z: [[f32; 3]; 3] = [[0.0, 0.0, 1.0]; 3];

    /// A triangle on the XY plane with UVs aligned to world axes yields a
    /// +X tangent with positive handedness on every vertex.
    #[test]
    fn axis_aligned_uv_maps_tangent_to_x() {
        let uvs = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let tangents = compute_tangents(&POSITIONS, &NORMALS_Z, &uvs, &[0, 1, 2]);

        assert_eq!(tangents.len(), 3);
        for t in &tangents {
            assert!((t[0] - 1.0).abs() < 1e-5, "tx should be 1, got {:?}", t);
            assert!(t[1].abs() < 1e-5 && t[2].abs() < 1e-5, "ty/tz should be 0, got {:?}", t);
            assert_eq!(t[3], 1.0);
        }
    }

    /// Mirrored UVs flip the bitangent relative to `cross(n, t)`: the
    /// tangent reverses direction and handedness becomes -1.
    #[test]
    fn mirrored_uvs_flip_handedness() {
        let uvs = [[0.0, 0.0], [-1.0, 0.0], [0.0, 1.0]];
        let tangents = compute_tangents(&POSITIONS, &NORMALS_Z, &uvs, &[0, 1, 2]);

        for t in &tangents {
            assert!((t[0] + 1.0).abs() < 1e-5, "tx should be -1, got {:?}", t);
            assert!(t[1].abs() < 1e-5 && t[2].abs() < 1e-5, "got {:?}", t);
            assert_eq!(t[3], -1.0);
        }
    }

    /// Identical UVs carry no directional information: every vertex falls
    /// back to the neutral `[1, 0, 0, 1]` frame instead of producing NaNs.
    #[test]
    fn degenerate_uvs_use_fallback() {
        let uvs = [[0.3, 0.7]; 3];
        let tangents = compute_tangents(&POSITIONS, &NORMALS_Z, &uvs, &[0, 1, 2]);

        assert_eq!(tangents, vec![[1.0, 0.0, 0.0, 1.0]; 3]);
    }

    /// Out-of-range indices are skipped silently and the output length
    /// always matches the vertex count.
    #[test]
    fn out_of_range_indices_are_skipped() {
        let uvs = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let tangents =
            compute_tangents(&POSITIONS, &NORMALS_Z, &uvs, &[0, 1, 2, 99, 99, 99]);

        assert_eq!(tangents.len(), 3);
    }

    /// Missing UV array entirely behaves like fully degenerate UVs.
    #[test]
    fn missing_uvs_use_fallback() {
        let tangents = compute_tangents(&POSITIONS, &NORMALS_Z, &[], &[0, 1, 2]);
        assert_eq!(tangents, vec![[1.0, 0.0, 0.0, 1.0]; 3]);
    }
}
