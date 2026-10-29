//! Collects level geometry into a [`NavGeometry`].

use redox_math::{Mat4, Vec3};

use crate::bake::NavGeometry;
use crate::data::NavAreaId;

/// Builds a [`NavGeometry`] from triangle meshes.
///
/// Each item is `(positions, indices, transform, area)`: `positions` are local vertices,
/// `indices` is a flat list of triangles (three per triangle), `transform` places the mesh in
/// the world. A transform with a negative determinant mirrors the mesh, so the winding of its
/// triangles is reversed to keep them facing the same way. Triangles that refer to a missing
/// vertex are skipped.
pub fn from_triangle_meshes<'a>(
    meshes: impl IntoIterator<Item = (&'a [[f32; 3]], &'a [u32], Mat4, NavAreaId)>,
) -> NavGeometry {
    let mut geometry = NavGeometry::default();
    for (positions, indices, transform, area) in meshes {
        let base = geometry.vertices.len() as u32;
        let mirrored = transform.determinant() < 0.0;
        for position in positions {
            let world = transform.transform_point3(Vec3::from_array(*position));
            geometry.vertices.push(world.to_array());
        }
        let vertex_count = positions.len() as u32;
        for triangle in indices.chunks_exact(3) {
            if triangle.iter().any(|&i| i >= vertex_count) {
                continue;
            }
            let (a, b, c) = (base + triangle[0], base + triangle[1], base + triangle[2]);
            let corners = if mirrored { [a, c, b] } else { [a, b, c] };
            geometry.triangles.push(corners);
            geometry.areas.push(area);
        }
    }
    geometry
}
