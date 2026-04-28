//! Offline asset pipeline: converts a standard [`Mesh`] into [`VGAssetData`].
//!
//! ## Algorithm
//!
//! 1. Extract triangles and compute their centroids.
//! 2. Sort triangles by a 3-D Morton code for spatial locality.
//! 3. Greedily fill meshlets: keep adding triangles until the meshlet would
//!    exceed [`MAX_VERTICES_PER_MESHLET`] or [`MAX_TRIANGLES_PER_MESHLET`].
//! 4. For each meshlet, compute a bounding sphere and backface cone.
//! 5. For LOD levels > 0, subsample triangles before re-clustering.

use std::collections::HashMap;
use crate::mesh::{Mesh, Vertex};
use super::meshlet::{
    MeshletDescriptor, VGAssetData, MAX_VERTICES_PER_MESHLET, MAX_TRIANGLES_PER_MESHLET,
};
use super::lod::LodChainConfig;

// ── Morton code ───────────────────────────────────────────────────────────────

fn expand_bits(v: u32) -> u64 {
    let v = v as u64 & 0x1fffff;
    let v = (v | (v << 32)) & 0x1f00000000ffff;
    let v = (v | (v << 16)) & 0x1f0000ff0000ff;
    let v = (v | (v << 8)) & 0x100f00f00f00f00f;
    let v = (v | (v << 4)) & 0x10c30c30c30c30c3;
    (v | (v << 2)) & 0x1249249249249249
}

fn morton3d(x: f32, y: f32, z: f32) -> u64 {
    let xi = (x.clamp(0.0, 1.0) * 2097151.0) as u32;
    let yi = (y.clamp(0.0, 1.0) * 2097151.0) as u32;
    let zi = (z.clamp(0.0, 1.0) * 2097151.0) as u32;
    expand_bits(xi) | (expand_bits(yi) << 1) | (expand_bits(zi) << 2)
}

/// Sort triangle indices by the Morton code of their centroids.
fn sort_triangles_spatially(centroids: &[[f32; 3]], aabb_min: [f32; 3], aabb_max: [f32; 3]) -> Vec<usize> {
    let extent = [
        (aabb_max[0] - aabb_min[0]).max(1e-6),
        (aabb_max[1] - aabb_min[1]).max(1e-6),
        (aabb_max[2] - aabb_min[2]).max(1e-6),
    ];
    let mut keyed: Vec<(u64, usize)> = centroids
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let nx = (c[0] - aabb_min[0]) / extent[0];
            let ny = (c[1] - aabb_min[1]) / extent[1];
            let nz = (c[2] - aabb_min[2]) / extent[2];
            (morton3d(nx, ny, nz), i)
        })
        .collect();
    keyed.sort_unstable_by_key(|&(k, _)| k);
    keyed.into_iter().map(|(_, i)| i).collect()
}

// ── Bounding sphere (Ritter's algorithm) ─────────────────────────────────────

fn compute_bounding_sphere(verts: &[Vertex]) -> ([f32; 3], f32) {
    if verts.is_empty() {
        return ([0.0; 3], 0.0);
    }

    // Initial guess: bounding box centre
    let mut bmin = verts[0].position;
    let mut bmax = verts[0].position;
    for v in verts.iter().skip(1) {
        for i in 0..3 {
            bmin[i] = bmin[i].min(v.position[i]);
            bmax[i] = bmax[i].max(v.position[i]);
        }
    }
    let mut center = [
        (bmin[0] + bmax[0]) * 0.5,
        (bmin[1] + bmax[1]) * 0.5,
        (bmin[2] + bmax[2]) * 0.5,
    ];

    // Find max distance from centre
    let mut radius = 0.0_f32;
    for v in verts {
        let d = dist2(center, v.position).sqrt();
        if d > radius {
            radius = d;
        }
    }

    // Expand to include all vertices
    for v in verts {
        let d = dist2(center, v.position).sqrt();
        if d > radius {
            let over = d - radius;
            let inv_d = 1.0 / d;
            center[0] += (v.position[0] - center[0]) * over * 0.5 * inv_d;
            center[1] += (v.position[1] - center[1]) * over * 0.5 * inv_d;
            center[2] += (v.position[2] - center[2]) * over * 0.5 * inv_d;
            radius = (radius + d) * 0.5;
        }
    }

    (center, radius)
}

#[inline]
fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    dx * dx + dy * dy + dz * dz
}

// ── Cone computation ─────────────────────────────────────────────────────────

/// Compute the backface cone for a meshlet.
/// Returns (axis, cutoff) where cutoff = cos(half_angle).
fn compute_cone(local_verts: &[Vertex], local_indices: &[u32]) -> ([f32; 3], f32) {
    let mut avg_normal = [0.0f32; 3];
    let tri_count = local_indices.len() / 3;

    for tri in 0..tri_count {
        let i0 = local_indices[tri * 3] as usize;
        let i1 = local_indices[tri * 3 + 1] as usize;
        let i2 = local_indices[tri * 3 + 2] as usize;

        if i0 >= local_verts.len() || i1 >= local_verts.len() || i2 >= local_verts.len() {
            continue;
        }

        let n0 = local_verts[i0].normal;
        let n1 = local_verts[i1].normal;
        let n2 = local_verts[i2].normal;

        avg_normal[0] += n0[0] + n1[0] + n2[0];
        avg_normal[1] += n0[1] + n1[1] + n2[1];
        avg_normal[2] += n0[2] + n1[2] + n2[2];
    }

    let len = (avg_normal[0] * avg_normal[0]
        + avg_normal[1] * avg_normal[1]
        + avg_normal[2] * avg_normal[2])
        .sqrt();

    if len < 1e-6 {
        return ([0.0, 0.0, 1.0], -1.0); // Degenerate: never cull
    }

    let axis = [
        avg_normal[0] / len,
        avg_normal[1] / len,
        avg_normal[2] / len,
    ];

    // Compute maximum deviation
    let mut max_cos: f32 = 1.0;
    for v in local_verts {
        let n = v.normal;
        let dot = n[0] * axis[0] + n[1] * axis[1] + n[2] * axis[2];
        max_cos = max_cos.min(dot);
    }

    (axis, max_cos - 0.1) // small bias for safety
}

// ── Global AABB ──────────────────────────────────────────────────────────────

fn compute_aabb(vertices: &[Vertex]) -> [[f32; 3]; 2] {
    if vertices.is_empty() {
        return [[0.0; 3]; 2];
    }
    let mut min = vertices[0].position;
    let mut max = vertices[0].position;
    for v in vertices.iter().skip(1) {
        for i in 0..3 {
            min[i] = min[i].min(v.position[i]);
            max[i] = max[i].max(v.position[i]);
        }
    }
    [min, max]
}

// ── LOD triangle subsampling ──────────────────────────────────────────────────

/// Produce a simplified index list by keeping every `1/fraction` triangle.
fn subsample_triangles(indices: &[u32], fraction: f32) -> Vec<u32> {
    if fraction >= 1.0 {
        return indices.to_vec();
    }
    let step = (1.0 / fraction).round() as usize;
    let step = step.max(2);
    let tri_count = indices.len() / 3;
    let mut out = Vec::with_capacity(tri_count / step * 3 + 3);
    for i in (0..tri_count).step_by(step) {
        out.push(indices[i * 3]);
        out.push(indices[i * 3 + 1]);
        out.push(indices[i * 3 + 2]);
    }
    out
}

// ── Core clustering ───────────────────────────────────────────────────────────

/// Cluster a triangle list into meshlets.
/// Returns (vertices, indices, descriptors) with offsets relative to this
/// asset's own slices (rebased on GPU upload).
fn cluster_triangles(
    source_verts: &[Vertex],
    source_indices: &[u32],
    lod_level: u32,
    lod_error: f32,
    material_index: u32,
) -> (Vec<Vertex>, Vec<u32>, Vec<MeshletDescriptor>) {
    let tri_count = source_indices.len() / 3;
    if tri_count == 0 {
        return (vec![], vec![], vec![]);
    }

    // Compute centroids and AABB
    let centroids: Vec<[f32; 3]> = (0..tri_count)
        .map(|i| {
            let v0 = source_verts[source_indices[i * 3] as usize].position;
            let v1 = source_verts[source_indices[i * 3 + 1] as usize].position;
            let v2 = source_verts[source_indices[i * 3 + 2] as usize].position;
            [
                (v0[0] + v1[0] + v2[0]) / 3.0,
                (v0[1] + v1[1] + v2[1]) / 3.0,
                (v0[2] + v1[2] + v2[2]) / 3.0,
            ]
        })
        .collect();

    let aabb = compute_aabb(source_verts);
    let sorted = sort_triangles_spatially(&centroids, aabb[0], aabb[1]);

    let mut all_verts: Vec<Vertex> = Vec::new();
    let mut all_indices: Vec<u32> = Vec::new();
    let mut meshlets: Vec<MeshletDescriptor> = Vec::new();

    let mut tri_cursor = 0;
    while tri_cursor < sorted.len() {
        let v_base = all_verts.len() as u32;
        let i_base = all_indices.len() as u32;

        let mut local_verts: Vec<Vertex> = Vec::new();
        let mut local_vert_map: HashMap<u32, u32> = HashMap::new();
        let mut tri_count_in_meshlet = 0usize;

        while tri_cursor < sorted.len() && tri_count_in_meshlet < MAX_TRIANGLES_PER_MESHLET {
            let tri_idx = sorted[tri_cursor];
            let orig = [
                source_indices[tri_idx * 3],
                source_indices[tri_idx * 3 + 1],
                source_indices[tri_idx * 3 + 2],
            ];

            // Count how many new vertices this triangle would add
            let new_count = orig.iter().filter(|&&v| !local_vert_map.contains_key(&v)).count();
            if local_verts.len() + new_count > MAX_VERTICES_PER_MESHLET {
                break;
            }

            for &v in &orig {
                if !local_vert_map.contains_key(&v) {
                    let local_idx = local_verts.len() as u32;
                    local_vert_map.insert(v, local_idx);
                    local_verts.push(source_verts[v as usize]);
                }
                // Store global index (v_base + local offset)
                all_indices.push(v_base + local_vert_map[&v]);
            }
            tri_count_in_meshlet += 1;
            tri_cursor += 1;
        }

        if local_verts.is_empty() {
            tri_cursor += 1; // safety to avoid infinite loop
            continue;
        }

        let index_count = all_indices.len() as u32 - i_base;

        // Compute bounding sphere and cone over local verts
        let (center, radius) = compute_bounding_sphere(&local_verts);
        let local_indices_for_cone: Vec<u32> = (0..local_verts.len() as u32).collect();
        let (cone_axis, cone_cutoff) = compute_cone(&local_verts, &local_indices_for_cone);

        meshlets.push(MeshletDescriptor {
            vertex_offset: v_base,
            vertex_count: local_verts.len() as u32,
            index_offset: i_base,
            index_count,
            sphere_center: center,
            sphere_radius: radius,
            cone_axis,
            cone_cutoff,
            lod_error,
            lod_level,
            material_index,
            _pad: 0,
        });

        all_verts.extend_from_slice(&local_verts);
    }

    (all_verts, all_indices, meshlets)
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Convert a standard mesh into virtual geometry asset data.
///
/// Generates meshlets for each LOD level defined in `lod_config`.
/// All LOD meshlets are concatenated into a single flat list;
/// the LOD level is encoded in [`MeshletDescriptor::lod_level`].
pub fn build_vg_asset(mesh: &Mesh, lod_config: &LodChainConfig, material_index: u32) -> VGAssetData {
    let mut all_verts: Vec<Vertex> = Vec::new();
    let mut all_indices: Vec<u32> = Vec::new();
    let mut all_meshlets: Vec<MeshletDescriptor> = Vec::new();

    for (lod_idx, lod) in lod_config.levels.iter().enumerate() {
        let lod_indices = subsample_triangles(&mesh.indices, lod.triangle_fraction);
        let lod_error = 1.0 - lod.triangle_fraction; // simple proxy

        let (verts, indices, mut meshlets) = cluster_triangles(
            &mesh.vertices,
            &lod_indices,
            lod_idx as u32,
            lod_error,
            material_index,
        );

        // Rebase offsets to be relative to the accumulated global slices
        let vert_base = all_verts.len() as u32;
        let idx_base = all_indices.len() as u32;
        for m in &mut meshlets {
            m.vertex_offset += vert_base;
            m.index_offset += idx_base;
        }

        all_verts.extend(verts);
        all_indices.extend(indices);
        all_meshlets.extend(meshlets);
    }

    let aabb = compute_aabb(&all_verts);
    VGAssetData {
        vertices: all_verts,
        indices: all_indices,
        meshlets: all_meshlets,
        aabb,
    }
}
