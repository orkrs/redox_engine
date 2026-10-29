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

// ── DAG cache (`.rdag`, version 2; the `.rmesh` format above is untouched) ───

use super::dag::{ClusterDag, DagCluster, DagParams, VGDagAsset};

const DAG_MAGIC: &[u8; 8] = b"RDAG\0\0\0\0";
const DAG_VERSION: u32 = 2;
/// Serialised size of one [`DagCluster`] (fields only, no padding).
const DAG_CLUSTER_BYTES: usize = 72;

/// Cache key of a DAG asset: the mesh key combined with the build parameters.
pub fn dag_cache_key(mesh: &Mesh, params: &DagParams, material_index: u32) -> u64 {
    let base = cache_key(mesh, &LodChainConfig::default(), material_index);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    base.hash(&mut h);
    params.group_size.hash(&mut h);
    params.target_ratio.to_bits().hash(&mut h);
    params.max_levels.hash(&mut h);
    h.finish()
}

fn dag_cache_path(key: u64) -> PathBuf {
    cache_dir().join(format!("{key:016x}.rdag"))
}

/// Load a baked DAG asset. Missing, corrupt or other-version files give `None`.
pub fn load_cached_dag(key: u64) -> Option<VGDagAsset> {
    let data = fs::read(dag_cache_path(key)).ok()?;
    decode_dag(&data).ok()
}

/// Write a baked DAG asset for a later boot (failures are ignored: the cache is optional).
pub fn store_cached_dag(key: u64, asset: &VGDagAsset) {
    let dir = cache_dir();
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let _ = fs::write(dag_cache_path(key), encode_dag(asset));
}

fn encode_dag(asset: &VGDagAsset) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(DAG_MAGIC);
    push_u32(&mut out, DAG_VERSION);
    push_u32(&mut out, asset.vertices.len() as u32);
    push_u32(&mut out, asset.indices.len() as u32);
    push_u32(&mut out, asset.dag.clusters.len() as u32);
    push_u32(&mut out, asset.dag.root_count);
    push_u32(&mut out, asset.material_index);
    for row in &asset.aabb {
        push_f32x3(&mut out, *row);
    }
    out.extend_from_slice(bytemuck::cast_slice(&asset.vertices));
    out.extend_from_slice(bytemuck::cast_slice(&asset.indices));
    for cluster in &asset.dag.clusters {
        encode_cluster(cluster, &mut out);
    }
    out
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_f32x3(out: &mut Vec<u8>, value: [f32; 3]) {
    for v in value {
        push_f32(out, v);
    }
}

fn encode_cluster(c: &DagCluster, out: &mut Vec<u8>) {
    let before = out.len();
    push_u32(out, c.index_offset);
    push_u32(out, c.index_count);
    push_f32x3(out, c.center);
    push_f32(out, c.radius);
    push_f32x3(out, c.lod_center);
    push_f32(out, c.lod_radius);
    push_f32(out, c.error);
    push_f32x3(out, c.parent_lod_center);
    push_f32(out, c.parent_lod_radius);
    push_f32(out, c.parent_error);
    push_u32(out, c.level);
    push_u32(out, c.group);
    debug_assert_eq!(out.len() - before, DAG_CLUSTER_BYTES);
}

fn invalid_data(msg: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg)
}

/// Bounds-checked little-endian reader over a cache file.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> std::io::Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.data.len())
            .ok_or_else(|| invalid_data("truncated rdag cache"))?;
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u32(&mut self) -> std::io::Result<u32> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| invalid_data("truncated rdag cache"))?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn f32(&mut self) -> std::io::Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }

    fn f32x3(&mut self) -> std::io::Result<[f32; 3]> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }
}

fn checked_bytes(count: usize, size: usize) -> std::io::Result<usize> {
    count
        .checked_mul(size)
        .ok_or_else(|| invalid_data("rdag size overflow"))
}

fn decode_dag(data: &[u8]) -> std::io::Result<VGDagAsset> {
    let mut r = Reader { data, pos: 0 };
    if r.take(8)? != &DAG_MAGIC[..] {
        return Err(invalid_data("bad rdag magic"));
    }
    if r.u32()? != DAG_VERSION {
        return Err(invalid_data("rdag version"));
    }
    let n_vert = r.u32()? as usize;
    let n_idx = r.u32()? as usize;
    let n_cluster = r.u32()? as usize;
    let root_count = r.u32()?;
    let material_index = r.u32()?;
    let aabb = [r.f32x3()?, r.f32x3()?];

    let vertex_bytes = r.take(checked_bytes(n_vert, std::mem::size_of::<Vertex>())?)?;
    let vertices: Vec<Vertex> = bytemuck::pod_collect_to_vec(vertex_bytes);
    let index_bytes = r.take(checked_bytes(n_idx, 4)?)?;
    let indices: Vec<u32> = bytemuck::pod_collect_to_vec(index_bytes);

    let mut clusters = Vec::new();
    for _ in 0..n_cluster {
        clusters.push(decode_cluster(&mut r)?);
    }
    Ok(VGDagAsset {
        vertices,
        indices,
        dag: ClusterDag {
            clusters,
            root_count,
        },
        aabb,
        material_index,
    })
}

fn decode_cluster(r: &mut Reader<'_>) -> std::io::Result<DagCluster> {
    Ok(DagCluster {
        index_offset: r.u32()?,
        index_count: r.u32()?,
        center: r.f32x3()?,
        radius: r.f32()?,
        lod_center: r.f32x3()?,
        lod_radius: r.f32()?,
        error: r.f32()?,
        parent_lod_center: r.f32x3()?,
        parent_lod_radius: r.f32()?,
        parent_error: r.f32()?,
        level: r.u32()?,
        group: r.u32()?,
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
