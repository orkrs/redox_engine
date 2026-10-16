//! Binary meshlet cache (`.cache/vg/*.rmesh`).
//!
//! Avoids re-running meshopt simplify/cluster on every boot for the same mesh.

use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use bytemuck::Zeroable;

use super::lod::LodChainConfig;
use super::meshlet::{MeshletDescriptor, VGAssetData};
use crate::mesh::{Mesh, Vertex};

const MAGIC: &[u8; 8] = b"RMESH1\0\0";
const VERSION: u32 = 1;

/// Cache key from source mesh + LOD chain + material index.
pub fn cache_key(mesh: &Mesh, lod: &LodChainConfig, material_index: u32) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    material_index.hash(&mut h);
    lod.levels.len().hash(&mut h);
    for level in &lod.levels {
        level.max_triangles.hash(&mut h);
        level.triangle_fraction.to_bits().hash(&mut h);
        level.max_relative_error.to_bits().hash(&mut h);
    }
    mesh.indices.len().hash(&mut h);
    mesh.vertices.len().hash(&mut h);
    for i in &mesh.indices {
        i.hash(&mut h);
    }
    for v in &mesh.vertices {
        v.position[0].to_bits().hash(&mut h);
        v.position[1].to_bits().hash(&mut h);
        v.position[2].to_bits().hash(&mut h);
        v.uv[0].to_bits().hash(&mut h);
        v.uv[1].to_bits().hash(&mut h);
    }
    h.finish()
}

fn cache_dir() -> PathBuf {
    PathBuf::from(".cache").join("vg")
}

fn cache_path(key: u64) -> PathBuf {
    cache_dir().join(format!("{key:016x}.rmesh"))
}

/// Load a previously baked [`VGAssetData`] if the file exists and matches.
pub fn load_cached(key: u64) -> Option<VGAssetData> {
    read_rmesh(&cache_path(key)).ok()
}

/// Write baked meshlets for a later boot.
pub fn store_cached(key: u64, data: &VGAssetData) {
    let dir = cache_dir();
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let _ = write_rmesh(&cache_path(key), data);
}

fn write_rmesh(path: &Path, data: &VGAssetData) -> std::io::Result<()> {
    let mut f = File::create(path)?;
    f.write_all(MAGIC)?;
    f.write_all(&VERSION.to_le_bytes())?;
    f.write_all(&(data.vertices.len() as u32).to_le_bytes())?;
    f.write_all(&(data.indices.len() as u32).to_le_bytes())?;
    f.write_all(&(data.meshlets.len() as u32).to_le_bytes())?;
    for i in 0..3 {
        f.write_all(&data.aabb[0][i].to_le_bytes())?;
    }
    for i in 0..3 {
        f.write_all(&data.aabb[1][i].to_le_bytes())?;
    }
    f.write_all(bytemuck::cast_slice(&data.vertices))?;
    f.write_all(bytemuck::cast_slice(&data.indices))?;
    f.write_all(bytemuck::cast_slice(&data.meshlets))?;
    Ok(())
}

fn read_rmesh(path: &Path) -> std::io::Result<VGAssetData> {
    let mut f = File::open(path)?;
    let mut magic = [0u8; 8];
    f.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "bad rmesh magic",
        ));
    }
    let mut u32b = [0u8; 4];
    f.read_exact(&mut u32b)?;
    if u32::from_le_bytes(u32b) != VERSION {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "rmesh version",
        ));
    }
    f.read_exact(&mut u32b)?;
    let n_vert = u32::from_le_bytes(u32b) as usize;
    f.read_exact(&mut u32b)?;
    let n_idx = u32::from_le_bytes(u32b) as usize;
    f.read_exact(&mut u32b)?;
    let n_mesh = u32::from_le_bytes(u32b) as usize;

    let mut aabb = [[0f32; 3]; 2];
    for row in &mut aabb {
        for c in row {
            let mut b = [0u8; 4];
            f.read_exact(&mut b)?;
            *c = f32::from_le_bytes(b);
        }
    }

    let mut vertices = vec![Vertex::zeroed(); n_vert];
    f.read_exact(bytemuck::cast_slice_mut(&mut vertices))?;
    let mut indices = vec![0u32; n_idx];
    f.read_exact(bytemuck::cast_slice_mut(&mut indices))?;
    let mut meshlets = vec![MeshletDescriptor::zeroed(); n_mesh];
    f.read_exact(bytemuck::cast_slice_mut(&mut meshlets))?;

    Ok(VGAssetData {
        vertices,
        indices,
        meshlets,
        aabb,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::Vertex;

    #[test]
    fn roundtrip_rmesh_tmp() {
        let data = VGAssetData {
            vertices: vec![Vertex {
                position: [1.0, 2.0, 3.0],
                normal: [0.0, 1.0, 0.0],
                uv: [0.5, 0.5],
                tangent: [1.0, 0.0, 0.0],
            }],
            indices: vec![0, 0, 0],
            meshlets: Vec::new(),
            aabb: [[0.0; 3], [1.0; 3]],
        };
        let dir = std::env::temp_dir().join("redox_rmesh_test");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("t.rmesh");
        write_rmesh(&path, &data).unwrap();
        let loaded = read_rmesh(&path).unwrap();
        assert_eq!(loaded.vertices.len(), 1);
        assert_eq!(loaded.indices, vec![0, 0, 0]);
        let _ = fs::remove_file(path);
    }
}
