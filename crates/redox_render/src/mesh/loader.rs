//! Simplified OBJ model loader using the `tobj` crate.
//!
//! Extracts positions, normals, and UVs (when available) and returns one
//! [`Mesh`] per object in the file.

use super::{Mesh, Vertex};
use std::path::Path;

/// Loads all meshes from an OBJ file.
///
/// Materials are ignored in the MVP — only geometry data is extracted.
///
/// # Errors
/// Returns a `tobj::LoadError` if the file cannot be opened or parsed.
pub fn load_obj<P: AsRef<Path>>(path: P) -> Result<Vec<Mesh>, tobj::LoadError> {
    let (models, _materials) = tobj::load_obj(
        path.as_ref(),
        &tobj::LoadOptions {
            triangulate: true,
            single_index: true,
            ..Default::default()
        },
    )?;

    let meshes = models
        .into_iter()
        .map(|model| {
            let m = model.mesh;
            let vertex_count = m.positions.len() / 3;
            let has_normals = m.normals.len() == m.positions.len();
            let has_uvs = m.texcoords.len() >= vertex_count * 2;

            // Collect attribute arrays first so tangents can be computed
            // over the whole indexed surface (Lengyel accumulation needs
            // triangle connectivity, not per-vertex data).
            let positions: Vec<[f32; 3]> = m
                .positions
                .chunks_exact(3)
                .map(|c| [c[0], c[1], c[2]])
                .collect();
            let normals: Vec<[f32; 3]> = if has_normals {
                m.normals
                    .chunks_exact(3)
                    .map(|c| [c[0], c[1], c[2]])
                    .collect()
            } else {
                vec![[0.0, 1.0, 0.0]; vertex_count]
            };
            let uvs: Vec<[f32; 2]> = if has_uvs {
                m.texcoords.chunks_exact(2).map(|c| [c[0], c[1]]).collect()
            } else {
                vec![[0.0, 0.0]; vertex_count]
            };

            let tangents = super::tangent::compute_tangents(&positions, &normals, &uvs, &m.indices);

            let vertices: Vec<Vertex> = (0..vertex_count)
                .map(|i| {
                    let t = tangents[i];
                    Vertex {
                        position: positions[i],
                        normal: normals[i],
                        uv: uvs[i],
                        // Vertex layout carries xyz only; handedness (w) is
                        // not part of `Vertex` yet.
                        tangent: [t[0], t[1], t[2]],
                    }
                })
                .collect();

            Mesh::new(vertices, m.indices)
        })
        .collect();

    Ok(meshes)
}
