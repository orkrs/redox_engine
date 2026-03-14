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

            let vertices: Vec<Vertex> = (0..vertex_count)
                .map(|i| {
                    let px = m.positions[i * 3];
                    let py = m.positions[i * 3 + 1];
                    let pz = m.positions[i * 3 + 2];

                    let (nx, ny, nz) = if has_normals {
                        (m.normals[i * 3], m.normals[i * 3 + 1], m.normals[i * 3 + 2])
                    } else {
                        (0.0, 1.0, 0.0) // default up normal
                    };

                    let (u, v) = if has_uvs {
                        (m.texcoords[i * 2], m.texcoords[i * 2 + 1])
                    } else {
                        (0.0, 0.0)
                    };

                    Vertex {
                        position: [px, py, pz],
                        normal: [nx, ny, nz],
                        uv: [u, v],
                        tangent: [1.0, 0.0, 0.0],
                    }
                })
                .collect();

            Mesh::new(vertices, m.indices)
        })
        .collect();

    Ok(meshes)
}
