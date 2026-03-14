//! Procedural generation of geometric primitives.
//!
//! Every function returns a [`Mesh`] with positions, normals, UVs, and indices
//! ready for upload to the GPU.

use super::{Mesh, Vertex};
use std::f32::consts::PI;

/// Creates a unit cube (side length = 1, centred at the origin).
///
/// Each face has its own vertices so that face normals are correct.
/// Total: 24 vertices, 36 indices.
pub fn create_cube() -> Mesh {
    #[rustfmt::skip]
    let vertices = vec![
        // Front  (+Z)
        Vertex { position: [-0.5, -0.5,  0.5], normal: [ 0.0,  0.0,  1.0], uv: [0.0, 1.0], tangent: [ 1.0,  0.0,  0.0] },
        Vertex { position: [ 0.5, -0.5,  0.5], normal: [ 0.0,  0.0,  1.0], uv: [1.0, 1.0], tangent: [ 1.0,  0.0,  0.0] },
        Vertex { position: [ 0.5,  0.5,  0.5], normal: [ 0.0,  0.0,  1.0], uv: [1.0, 0.0], tangent: [ 1.0,  0.0,  0.0] },
        Vertex { position: [-0.5,  0.5,  0.5], normal: [ 0.0,  0.0,  1.0], uv: [0.0, 0.0], tangent: [ 1.0,  0.0,  0.0] },
        // Back   (-Z)
        Vertex { position: [ 0.5, -0.5, -0.5], normal: [ 0.0,  0.0, -1.0], uv: [0.0, 1.0], tangent: [-1.0,  0.0,  0.0] },
        Vertex { position: [-0.5, -0.5, -0.5], normal: [ 0.0,  0.0, -1.0], uv: [1.0, 1.0], tangent: [-1.0,  0.0,  0.0] },
        Vertex { position: [-0.5,  0.5, -0.5], normal: [ 0.0,  0.0, -1.0], uv: [1.0, 0.0], tangent: [-1.0,  0.0,  0.0] },
        Vertex { position: [ 0.5,  0.5, -0.5], normal: [ 0.0,  0.0, -1.0], uv: [0.0, 0.0], tangent: [-1.0,  0.0,  0.0] },
        // Top    (+Y)
        Vertex { position: [-0.5,  0.5,  0.5], normal: [ 0.0,  1.0,  0.0], uv: [0.0, 1.0], tangent: [ 1.0,  0.0,  0.0] },
        Vertex { position: [ 0.5,  0.5,  0.5], normal: [ 0.0,  1.0,  0.0], uv: [1.0, 1.0], tangent: [ 1.0,  0.0,  0.0] },
        Vertex { position: [ 0.5,  0.5, -0.5], normal: [ 0.0,  1.0,  0.0], uv: [1.0, 0.0], tangent: [ 1.0,  0.0,  0.0] },
        Vertex { position: [-0.5,  0.5, -0.5], normal: [ 0.0,  1.0,  0.0], uv: [0.0, 0.0], tangent: [ 1.0,  0.0,  0.0] },
        // Bottom (-Y)
        Vertex { position: [-0.5, -0.5, -0.5], normal: [ 0.0, -1.0,  0.0], uv: [0.0, 1.0], tangent: [ 1.0,  0.0,  0.0] },
        Vertex { position: [ 0.5, -0.5, -0.5], normal: [ 0.0, -1.0,  0.0], uv: [1.0, 1.0], tangent: [ 1.0,  0.0,  0.0] },
        Vertex { position: [ 0.5, -0.5,  0.5], normal: [ 0.0, -1.0,  0.0], uv: [1.0, 0.0], tangent: [ 1.0,  0.0,  0.0] },
        Vertex { position: [-0.5, -0.5,  0.5], normal: [ 0.0, -1.0,  0.0], uv: [0.0, 0.0], tangent: [ 1.0,  0.0,  0.0] },
        // Right  (+X)
        Vertex { position: [ 0.5, -0.5,  0.5], normal: [ 1.0,  0.0,  0.0], uv: [0.0, 1.0], tangent: [ 0.0,  0.0, -1.0] },
        Vertex { position: [ 0.5, -0.5, -0.5], normal: [ 1.0,  0.0,  0.0], uv: [1.0, 1.0], tangent: [ 0.0,  0.0, -1.0] },
        Vertex { position: [ 0.5,  0.5, -0.5], normal: [ 1.0,  0.0,  0.0], uv: [1.0, 0.0], tangent: [ 0.0,  0.0, -1.0] },
        Vertex { position: [ 0.5,  0.5,  0.5], normal: [ 1.0,  0.0,  0.0], uv: [0.0, 0.0], tangent: [ 0.0,  0.0, -1.0] },
        // Left   (-X)
        Vertex { position: [-0.5, -0.5, -0.5], normal: [-1.0,  0.0,  0.0], uv: [0.0, 1.0], tangent: [ 0.0,  0.0,  1.0] },
        Vertex { position: [-0.5, -0.5,  0.5], normal: [-1.0,  0.0,  0.0], uv: [1.0, 1.0], tangent: [ 0.0,  0.0,  1.0] },
        Vertex { position: [-0.5,  0.5,  0.5], normal: [-1.0,  0.0,  0.0], uv: [1.0, 0.0], tangent: [ 0.0,  0.0,  1.0] },
        Vertex { position: [-0.5,  0.5, -0.5], normal: [-1.0,  0.0,  0.0], uv: [0.0, 0.0], tangent: [ 0.0,  0.0,  1.0] },
    ];

    let mut indices = Vec::with_capacity(36);
    for face in 0..6u32 {
        let base = face * 4;
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    Mesh::new(vertices, indices)
}

/// Creates a unit cube with normals pointing **inward** and winding reversed so that
/// when viewed from inside (e.g. room walls), faces are front-facing and lit correctly.
/// Use for interior geometry (corridor walls, floor, ceiling) with back-face culling.
pub fn create_cube_inward_normals() -> Mesh {
    let mut mesh = create_cube();
    for v in &mut mesh.vertices {
        v.normal = [-v.normal[0], -v.normal[1], -v.normal[2]];
        v.tangent = [-v.tangent[0], -v.tangent[1], -v.tangent[2]];
    }
    // Reverse winding so the interior face is front when viewed from inside
    for tri in mesh.indices.chunks_exact_mut(3) {
        tri.swap(1, 2);
    }
    mesh
}

/// Creates a UV sphere with the given `radius` and resolution.
///
/// * `segments` — number of horizontal slices (longitude).
/// * `rings` is derived as `segments / 2`.
///
/// Total vertices ≈ `(segments + 1) * (rings + 1)`.
pub fn create_sphere(radius: f32, segments: u32) -> Mesh {
    let rings = segments / 2;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    for ring in 0..=rings {
        let theta = PI * ring as f32 / rings as f32;
        let sin_theta = theta.sin();
        let cos_theta = theta.cos();

        for seg in 0..=segments {
            let phi = 2.0 * PI * seg as f32 / segments as f32;
            let sin_phi = phi.sin();
            let cos_phi = phi.cos();

            let x = cos_phi * sin_theta;
            let y = cos_theta;
            let z = sin_phi * sin_theta;

            let u = seg as f32 / segments as f32;
            let v = ring as f32 / rings as f32;

            let tx = -sin_phi;
            let ty = 0.0;
            let tz = cos_phi;

            vertices.push(Vertex {
                position: [x * radius, y * radius, z * radius],
                normal: [x, y, z],
                uv: [u, v],
                tangent: [tx, ty, tz],
            });
        }
    }

    for ring in 0..rings {
        for seg in 0..segments {
            let current = ring * (segments + 1) + seg;
            let next = current + segments + 1;

            indices.extend_from_slice(&[current, next, current + 1]);
            indices.extend_from_slice(&[current + 1, next, next + 1]);
        }
    }

    Mesh::new(vertices, indices)
}

/// Creates a torus lying in the XZ plane.
///
/// * `radius`      — distance from centre of the torus to centre of the tube.
/// * `tube_radius` — radius of the tube itself.
/// * `segments`    — resolution around the main ring.
/// * `tube_segments` — resolution around the tube cross‑section.
pub fn create_torus(radius: f32, tube_radius: f32, segments: u32, tube_segments: u32) -> Mesh {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    for seg in 0..=segments {
        let theta = 2.0 * PI * seg as f32 / segments as f32;
        let cos_theta = theta.cos();
        let sin_theta = theta.sin();

        for tube in 0..=tube_segments {
            let phi = 2.0 * PI * tube as f32 / tube_segments as f32;
            let cos_phi = phi.cos();
            let sin_phi = phi.sin();

            let x = (radius + tube_radius * cos_phi) * cos_theta;
            let y = tube_radius * sin_phi;
            let z = (radius + tube_radius * cos_phi) * sin_theta;

            let nx = cos_phi * cos_theta;
            let ny = sin_phi;
            let nz = cos_phi * sin_theta;

            let u = seg as f32 / segments as f32;
            let v = tube as f32 / tube_segments as f32;

            let tx = -sin_theta;
            let ty = 0.0;
            let tz = cos_theta;

            vertices.push(Vertex {
                position: [x, y, z],
                normal: [nx, ny, nz],
                uv: [u, v],
                tangent: [tx, ty, tz],
            });
        }
    }

    for seg in 0..segments {
        for tube in 0..tube_segments {
            let current = seg * (tube_segments + 1) + tube;
            let next = (seg + 1) * (tube_segments + 1) + tube;

            indices.extend_from_slice(&[current, next, current + 1]);
            indices.extend_from_slice(&[current + 1, next, next + 1]);
        }
    }

    Mesh::new(vertices, indices)
}

/// Creates a unit quad in the XY plane (normal pointing +Z).
///
/// Useful for debugging textures and full-screen passes.
/// Total: 4 vertices, 6 indices.
pub fn create_quad() -> Mesh {
    let vertices = vec![
        Vertex {
            position: [-0.5, -0.5, 0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 1.0],
            tangent: [1.0, 0.0, 0.0],
        },
        Vertex {
            position: [0.5, -0.5, 0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [1.0, 1.0],
            tangent: [1.0, 0.0, 0.0],
        },
        Vertex {
            position: [0.5, 0.5, 0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [1.0, 0.0],
            tangent: [1.0, 0.0, 0.0],
        },
        Vertex {
            position: [-0.5, 0.5, 0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 0.0],
            tangent: [1.0, 0.0, 0.0],
        },
    ];
    let indices = vec![0, 1, 2, 0, 2, 3];
    Mesh::new(vertices, indices)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_vertex_index_counts() {
        let cube = create_cube();
        assert_eq!(
            cube.vertices.len(),
            24,
            "Cube must have 24 vertices (4 per face × 6 faces)"
        );
        assert_eq!(
            cube.indices.len(),
            36,
            "Cube must have 36 indices (6 per face × 6 faces)"
        );
    }

    #[test]
    fn sphere_vertex_index_counts() {
        let sphere = create_sphere(1.0, 16);
        let rings = 16u32 / 2;
        let expected_verts = ((16 + 1) * (rings + 1)) as usize;
        assert_eq!(sphere.vertices.len(), expected_verts);
        let expected_indices = (16 * rings * 6) as usize;
        assert_eq!(sphere.indices.len(), expected_indices);
    }

    #[test]
    fn quad_vertex_index_counts() {
        let quad = create_quad();
        assert_eq!(quad.vertices.len(), 4);
        assert_eq!(quad.indices.len(), 6);
    }

    #[test]
    fn torus_has_valid_geometry() {
        let torus = create_torus(1.0, 0.3, 16, 8);
        assert!(!torus.vertices.is_empty());
        assert!(!torus.indices.is_empty());
        // Verify all indices are in-bounds.
        let max_idx = torus.vertices.len() as u32;
        for &idx in &torus.indices {
            assert!(idx < max_idx, "Index {idx} out of bounds (max {max_idx})");
        }
    }

    #[test]
    fn aabb_is_computed() {
        let cube = create_cube();
        assert!(cube.aabb.min.x <= -0.49);
        assert!(cube.aabb.max.x >= 0.49);
    }
}
