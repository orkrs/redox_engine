//! Offline asset pipeline: converts a standard [`Mesh`] into [`VGAssetData`]
//! using the `meshopt` (meshoptimizer) library.
//!
//! ## Algorithm (per LOD level `L` from [`LodChainConfig`])
//!
//! 1. **Simplify** — [`meshopt::simplify()`](meshopt::simplify()) (quadric-error decimation) reduces
//!    the triangle count towards `triangle_fraction × source_triangles`.
//!    Each level is simplified from the previous one so the LOD chain stays
//!    topologically consistent. The result is watertight — unlike triangle
//!    subsampling, no holes are ever produced. Vertices never move; only the
//!    index buffer shrinks.
//! 2. **Cluster** — [`meshopt::build_meshlets`] partitions the level into
//!    meshlets respecting [`MAX_VERTICES_PER_MESHLET`] /
//!    [`MAX_TRIANGLES_PER_MESHLET`] (both within meshopt's hard limits of
//!    256 vertices / 512 triangles, with triangle count divisible by 4).
//! 3. **Bounds** — [`meshopt::compute_meshlet_bounds`] provides the bounding
//!    sphere and backface cone for every meshlet.
//!
//! Meshlet vertices are direct copies of the *original* source vertices, so
//! vertex attributes (normals, UVs, tangents) are never re-interpolated —
//! only topology changes across LODs. Every descriptor is tagged with its
//! `lod_level`; the runtime picks one level per instance and culls/draws
//! only meshlets of that level.

use meshopt::{
    SimplifyOptions, VertexDataAdapter, build_meshlets, compute_meshlet_bounds, simplify,
    simplify_scale,
};

use super::lod::LodChainConfig;
use super::meshlet::{
    MAX_TRIANGLES_PER_MESHLET, MAX_VERTICES_PER_MESHLET, MeshletDescriptor, VGAssetData,
};
use crate::mesh::{Mesh, Vertex};

/// Weight for cone tightness during meshlet clustering. 0.0 = purely spatial
/// fit; larger values produce tighter backface cones (better culling) at a
/// small packing cost. 0.5 matches the meshoptimizer sample default.
const MESHLET_CONE_WEIGHT: f32 = 0.5;

// ── meshopt adapters ─────────────────────────────────────────────────────────

/// Byte view over a vertex pool for meshopt. `Vertex.position` sits at byte
/// offset 0 (see `Vertex::buffer_layout`).
pub(super) fn position_adapter(verts: &[Vertex]) -> VertexDataAdapter<'_> {
    VertexDataAdapter::new(
        bytemuck::cast_slice(verts),
        std::mem::size_of::<Vertex>(),
        std::mem::offset_of!(Vertex, position),
    )
    .expect("vertex slice is evenly divisible by the vertex stride")
}

// ── Simplification ───────────────────────────────────────────────────────────

/// Reduce `indices` to at most `target_triangles` triangles via quadric-error
/// simplification. Returns the (possibly unchanged) index buffer plus the
/// resulting geometric error in world-space units.
///
/// `max_relative_error` bounds the simplification error relative to the mesh
/// extent; a generous bound lets the decimation reach the target count, a
/// tight one trades triangle count for quality.
fn simplify_level(
    verts: &[Vertex],
    indices: &[u32],
    target_triangles: usize,
    max_relative_error: f32,
) -> (Vec<u32>, f32) {
    let tri_count = indices.len() / 3;
    if tri_count == 0 || target_triangles >= tri_count {
        return (indices.to_vec(), 0.0);
    }

    let adapter = position_adapter(verts);
    // `target_count` must be a multiple of 3 (one per triangle).
    let target_count = (target_triangles.max(1) * 3).min(tri_count * 3);
    let error_bound = max_relative_error.clamp(1e-6, 1.0);
    let mut result_error = 0.0_f32;

    // LockBorder pins topological border vertices so open meshes (terrain
    // chunks, patches) keep a stable silhouette across LODs; it is a no-op
    // for closed meshes.
    let simplified = simplify(
        indices,
        &adapter,
        target_count,
        error_bound,
        SimplifyOptions::LockBorder,
        Some(&mut result_error),
    );

    // meshopt reports error relative to the mesh extent; the descriptor wants
    // absolute world-space units.
    let absolute_error = result_error * simplify_scale(&adapter);
    (simplified, absolute_error)
}

// ── Clustering ───────────────────────────────────────────────────────────────

/// Cluster one LOD level into meshlets, appending vertices, indices and
/// descriptors to the accumulators. Offsets are rebased onto the running
/// asset-wide slices.
fn cluster_level(
    level_verts: &[Vertex],
    level_indices: &[u32],
    lod_level: u32,
    lod_error: f32,
    material_index: u32,
    out_verts: &mut Vec<Vertex>,
    out_indices: &mut Vec<u32>,
    out_meshlets: &mut Vec<MeshletDescriptor>,
) {
    if level_indices.is_empty() {
        return;
    }

    let adapter = position_adapter(level_verts);
    let built = build_meshlets(
        level_indices,
        &adapter,
        MAX_VERTICES_PER_MESHLET,
        MAX_TRIANGLES_PER_MESHLET,
        MESHLET_CONE_WEIGHT,
    );

    for meshlet in built.iter() {
        if meshlet.triangles.is_empty() {
            continue;
        }

        let vert_base = out_verts.len() as u32;
        // `meshlet.vertices` lists each unique source vertex exactly once,
        // already order-optimised by meshopt.
        for &src in meshlet.vertices {
            out_verts.push(level_verts[src as usize]);
        }

        let index_base = out_indices.len() as u32;
        // `meshlet.triangles` is a micro index buffer of u8 triplets whose
        // values are slots into `meshlet.vertices`; after appending those
        // vertices in order, a slot maps to `vert_base + slot`.
        for &slot in meshlet.triangles {
            out_indices.push(vert_base + slot as u32);
        }

        let bounds = compute_meshlet_bounds(meshlet, &adapter);

        out_meshlets.push(MeshletDescriptor {
            vertex_offset: vert_base,
            vertex_count: meshlet.vertices.len() as u32,
            index_offset: index_base,
            index_count: meshlet.triangles.len() as u32,
            sphere_center: bounds.center,
            sphere_radius: bounds.radius.max(0.0),
            cone_axis: bounds.cone_axis,
            cone_cutoff: bounds.cone_cutoff.clamp(-1.0, 1.0),
            lod_error,
            lod_level,
            material_index,
            _pad: 0,
        });
    }
}

// ── Global AABB ──────────────────────────────────────────────────────────────

pub(super) fn compute_aabb(vertices: &[Vertex]) -> [[f32; 3]; 2] {
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

// ── Public entry point ────────────────────────────────────────────────────────

/// Convert a standard mesh into virtual geometry asset data.
///
/// Generates meshlets for each LOD level defined in `lod_config`. All LOD
/// meshlets are concatenated into a single flat list; the LOD level is
/// encoded in [`MeshletDescriptor::lod_level`].
pub fn build_vg_asset(
    mesh: &Mesh,
    lod_config: &LodChainConfig,
    material_index: u32,
) -> VGAssetData {
    let mut all_verts: Vec<Vertex> = Vec::new();
    let mut all_indices: Vec<u32> = Vec::new();
    let mut all_meshlets: Vec<MeshletDescriptor> = Vec::new();

    let source_tris = mesh.indices.len() / 3;
    if source_tris == 0 {
        return VGAssetData {
            vertices: all_verts,
            indices: all_indices,
            meshlets: all_meshlets,
            aabb: [[0.0; 3]; 2],
        };
    }

    // Each level is simplified from the previous one (chained), which keeps
    // the LOD chain topologically consistent. `current_tris` tracks the
    // previous level's *actual* count so successive targets can never grow.
    let mut current: Vec<u32> = mesh.indices.clone();
    let mut current_tris = source_tris;

    for (level_idx, level) in lod_config.levels.iter().enumerate() {
        let fraction = if level.triangle_fraction.is_finite() {
            level.triangle_fraction
        } else {
            1.0
        };

        let target_tris = if level_idx == 0 || fraction >= 1.0 {
            current_tris
        } else {
            ((source_tris as f32 * fraction).round() as usize)
                .clamp(1, current_tris.saturating_sub(1))
        };

        let (indices, lod_error) = simplify_level(
            &mesh.vertices,
            &current,
            target_tris,
            level.max_relative_error,
        );

        cluster_level(
            &mesh.vertices,
            &indices,
            level_idx as u32,
            lod_error,
            material_index,
            &mut all_verts,
            &mut all_indices,
            &mut all_meshlets,
        );

        current_tris = indices.len() / 3;
        current = indices;
    }

    // All levels reference copies of the original vertices, so the AABB of
    // the source vertex pool covers every level.
    let aabb = compute_aabb(&mesh.vertices);
    VGAssetData {
        vertices: all_verts,
        indices: all_indices,
        meshlets: all_meshlets,
        aabb,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::Vertex;
    use std::f32::consts::PI;

    fn vertex(position: [f32; 3], normal: [f32; 3]) -> Vertex {
        Vertex {
            position,
            normal,
            uv: [0.0, 0.0],
            tangent: [1.0, 0.0, 0.0],
        }
    }

    /// Unit cube (12 triangles, closed manifold).
    fn cube_mesh() -> Mesh {
        let corners: [[f32; 3]; 8] = [
            [-1.0, -1.0, -1.0],
            [1.0, -1.0, -1.0],
            [1.0, 1.0, -1.0],
            [-1.0, 1.0, -1.0],
            [-1.0, -1.0, 1.0],
            [1.0, -1.0, 1.0],
            [1.0, 1.0, 1.0],
            [-1.0, 1.0, 1.0],
        ];
        let faces: [[usize; 4]; 6] = [
            [0, 3, 2, 1], // -z
            [4, 5, 6, 7], // +z
            [0, 1, 5, 4], // -y
            [2, 3, 7, 6], // +y
            [1, 2, 6, 5], // +x
            [0, 4, 7, 3], // -x
        ];
        let mut verts: Vec<Vertex> = corners.iter().map(|&p| vertex(p, p)).collect();
        verts.resize(24, vertex([0.0; 3], [0.0; 3])); // placeholder, replaced below
        // Flat-shaded cube: 4 unique vertices per face for clean normals.
        let mut verts: Vec<Vertex> = Vec::with_capacity(24);
        let mut indices: Vec<u32> = Vec::with_capacity(36);
        for face in &faces {
            let base = verts.len() as u32;
            let n = {
                let a = corners[face[1]];
                let b = corners[face[0]];
                let c = corners[face[2]];
                let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                [
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ]
            };
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            let n = [n[0] / len, n[1] / len, n[2] / len];
            for &c in face {
                verts.push(vertex(corners[c], n));
            }
            indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        Mesh::new(verts, indices)
    }

    /// UV-sphere with `rings` latitude rows and `segments` longitude columns.
    /// `rings = 40, segments = 64` → 4992 triangles (~5k).
    fn uv_sphere(radius: f32, rings: usize, segments: usize) -> Mesh {
        let row_len = segments + 1;
        let mut verts: Vec<Vertex> = Vec::with_capacity((rings + 1) * row_len);
        for r in 0..=rings {
            let phi = PI * r as f32 / rings as f32;
            let (sp, cp) = phi.sin_cos();
            for s in 0..=segments {
                let theta = 2.0 * PI * s as f32 / segments as f32;
                let (st, ct) = theta.sin_cos();
                let n = [sp * ct, cp, sp * st];
                verts.push(vertex([n[0] * radius, n[1] * radius, n[2] * radius], n));
            }
        }

        let mut indices: Vec<u32> = Vec::new();
        for r in 0..rings {
            for s in 0..segments {
                let a = (r * row_len + s) as u32;
                let b = a + 1;
                let c = a + row_len as u32;
                let d = c + 1;
                // Skip degenerate pole triangles (a == b at north, c == d at
                // south pole rows).
                if r > 0 {
                    indices.extend([a, b, d]);
                }
                if r < rings - 1 {
                    indices.extend([a, d, c]);
                }
            }
        }
        Mesh::new(verts, indices)
    }

    fn tri_area(verts: &[Vertex], i0: u32, i1: u32, i2: u32) -> f32 {
        let a = verts[i0 as usize].position;
        let b = verts[i1 as usize].position;
        let c = verts[i2 as usize].position;
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let cx = u[1] * v[2] - u[2] * v[1];
        let cy = u[2] * v[0] - u[0] * v[2];
        let cz = u[0] * v[1] - u[1] * v[0];
        0.5 * (cx * cx + cy * cy + cz * cz).sqrt()
    }

    /// Shared invariants: caps, LOD tags, index ranges, no degenerate
    /// triangles. Returns per-level (vertex, triangle) counts.
    fn check_asset(asset: &VGAssetData, level_count: usize) -> Vec<(usize, usize)> {
        let mut per_level: Vec<(usize, usize)> = vec![(0, 0); level_count];

        for m in &asset.meshlets {
            assert!(
                (m.lod_level as usize) < level_count,
                "lod_level out of range"
            );
            assert_eq!(
                m.index_count % 3,
                0,
                "meshlet index count must be a multiple of 3"
            );
            assert!(
                m.vertex_count as usize <= MAX_VERTICES_PER_MESHLET,
                "vertex cap violated: {}",
                m.vertex_count
            );
            assert!(
                (m.index_count / 3) as usize <= MAX_TRIANGLES_PER_MESHLET,
                "triangle cap violated: {}",
                m.index_count / 3
            );

            // Every index must reference a vertex inside the meshlet's own range.
            let start = m.index_offset as usize;
            let end = start + m.index_count as usize;
            for &idx in &asset.indices[start..end] {
                assert!(
                    idx >= m.vertex_offset && idx < m.vertex_offset + m.vertex_count,
                    "index {} escapes meshlet vertex range [{}, {})",
                    idx,
                    m.vertex_offset,
                    m.vertex_offset + m.vertex_count
                );
            }

            // No degenerate triangles at any LOD level.
            let tstart = m.index_offset as usize;
            for t in (0..m.index_count as usize).step_by(3) {
                let area = tri_area(
                    &asset.vertices,
                    asset.indices[tstart + t],
                    asset.indices[tstart + t + 1],
                    asset.indices[tstart + t + 2],
                );
                assert!(
                    area > 1e-10,
                    "degenerate triangle (area {area}) in LOD {}",
                    m.lod_level
                );
            }

            let slot = &mut per_level[m.lod_level as usize];
            slot.0 += m.vertex_count as usize;
            slot.1 += (m.index_count / 3) as usize;
        }
        per_level
    }

    #[test]
    fn cube_builds_valid_lod_chain() {
        let mesh = cube_mesh();
        assert_eq!(mesh.indices.len() / 3, 12);
        let cfg = LodChainConfig::default();
        let asset = build_vg_asset(&mesh, &cfg, 0);

        let per_level = check_asset(&asset, cfg.levels.len());
        assert_eq!(
            asset.triangle_count() as usize,
            per_level.iter().map(|p| p.1).sum::<usize>()
        );

        // Every level must be present.
        for (lvl, (v, t)) in per_level.iter().enumerate() {
            assert!(*t > 0, "LOD {lvl} produced no triangles");
            assert!(*v > 0, "LOD {lvl} produced no vertices");
        }

        // Counts must not grow across levels.
        for w in per_level.windows(2) {
            assert!(w[0].1 >= w[1].1, "triangle count grew across LODs: {:?}", w);
            assert!(w[0].0 >= w[1].0, "vertex count grew across LODs: {:?}", w);
        }
    }

    #[test]
    fn sphere_lods_shrink_monotonically_without_holes() {
        let mesh = uv_sphere(1.0, 40, 64);
        let src_tris = mesh.indices.len() / 3;
        assert_eq!(src_tris, 4992);

        let cfg = LodChainConfig::default();
        let asset = build_vg_asset(&mesh, &cfg, 7);

        let per_level = check_asset(&asset, cfg.levels.len());

        // LOD 0 is the untouched source.
        assert_eq!(per_level[0].1, src_tris);

        // Strict shrinkage for a dense sphere (quadric decimation reaches its
        // targets comfortably).
        assert!(
            per_level[1].1 < per_level[0].1,
            "LOD 1 did not shrink: {} vs {}",
            per_level[1].1,
            per_level[0].1
        );
        assert!(
            per_level[2].1 < per_level[1].1,
            "LOD 2 did not shrink: {} vs {}",
            per_level[2].1,
            per_level[1].1
        );
        // And it actually decimates towards the configured fractions.
        assert!(per_level[1].1 as f32 <= src_tris as f32 * 0.55);
        assert!(per_level[2].1 as f32 <= src_tris as f32 * 0.30);

        // AABB covers the unit sphere.
        for axis in 0..3 {
            assert!((asset.aabb[0][axis] + 1.0).abs() < 1e-4);
            assert!((asset.aabb[1][axis] - 1.0).abs() < 1e-4);
        }
    }

    #[test]
    fn empty_mesh_produces_empty_asset() {
        let mesh = Mesh::new(vec![], vec![]);
        let cfg = LodChainConfig::default();
        let asset = build_vg_asset(&mesh, &cfg, 0);
        assert!(asset.vertices.is_empty());
        assert!(asset.indices.is_empty());
        assert!(asset.meshlets.is_empty());
        assert_eq!(asset.triangle_count(), 0);
    }

    #[test]
    fn single_level_chain_is_identity() {
        let mesh = uv_sphere(1.0, 8, 12);
        let src_tris = mesh.indices.len() / 3;
        let cfg = LodChainConfig {
            levels: vec![super::super::lod::LodLevelConfig::default()],
        };
        let asset = build_vg_asset(&mesh, &cfg, 3);
        let per_level = check_asset(&asset, 1);
        assert_eq!(per_level[0].1, src_tris);
        assert!(asset.meshlets.iter().all(|m| m.lod_error == 0.0));
        assert!(asset.meshlets.iter().all(|m| m.material_index == 3));
    }
}
