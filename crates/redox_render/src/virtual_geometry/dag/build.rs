//! Construction of the cluster hierarchy with meshopt (simplify + meshlets).

use std::collections::HashMap;

use meshopt::{
    SimplifyOptions, VertexDataAdapter, build_meshlets, compute_meshlet_bounds, simplify,
    simplify_scale,
};

use super::group::{adjacency, group_clusters};
use super::{ClusterDag, DagCluster, DagError, DagParams, dist, validate};
use crate::mesh::Vertex;
use crate::virtual_geometry::asset_pipeline::position_adapter;
use crate::virtual_geometry::meshlet::{MAX_TRIANGLES_PER_MESHLET, MAX_VERTICES_PER_MESHLET};

/// `group` value of leaves.
const NO_GROUP: u32 = u32::MAX;
/// Cone weight for leaf clustering (purely spatial groups).
const CONE_WEIGHT: f32 = 0.0;
/// meshopt target error for group simplification; the index target governs the result.
const GROUP_TARGET_ERROR: f32 = 1.0;
/// A group keeping at least this share of its indices is treated as stuck.
const STUCK_SHARE: f32 = 0.9;
/// Relative safety margin added to a group error so it stays strictly above the children errors.
const ERROR_MARGIN_REL: f32 = 1e-3;
/// Absolute safety margin added to a group error.
const ERROR_MARGIN_ABS: f32 = 1e-6;
/// Relative margin added to group LOD sphere radii (keeps children strictly inside).
const SPHERE_MARGIN_REL: f32 = 1e-5;

/// Position-based welding: `weld[i]` is the canonical (first) vertex with the same position.
/// `-0.0` and `+0.0` are the same position.
pub fn weld_positions(vertices: &[Vertex]) -> Vec<u32> {
    let mut seen: HashMap<[u32; 3], u32> = HashMap::with_capacity(vertices.len());
    vertices
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let key = v.position.map(canonical_bits);
            *seen.entry(key).or_insert(i as u32)
        })
        .collect()
}

fn canonical_bits(x: f32) -> u32 {
    if x == 0.0 {
        0.0f32.to_bits()
    } else {
        x.to_bits()
    }
}

/// Bounding sphere used as LOD sphere of a group.
#[derive(Clone, Copy)]
struct Sphere {
    center: [f32; 3],
    radius: f32,
}

/// Mutable state of one hierarchy build.
struct DagBuilder {
    weld: Vec<u32>,
    clusters: Vec<DagCluster>,
    indices: Vec<u32>,
}

impl DagBuilder {
    /// Appends the meshlets of `tris` (canonical vertex ids, three per triangle) as clusters.
    /// `lod` is the group sphere; `None` (leaves) uses each cluster's own bounds.
    fn emit(
        &mut self,
        adapter: &VertexDataAdapter<'_>,
        tris: &[u32],
        level: u32,
        group: u32,
        lod: Option<Sphere>,
        error: f32,
    ) -> Vec<u32> {
        let built = build_meshlets(
            tris,
            adapter,
            MAX_VERTICES_PER_MESHLET,
            MAX_TRIANGLES_PER_MESHLET,
            CONE_WEIGHT,
        );
        let mut ids = Vec::with_capacity(built.len());
        for meshlet in built.iter() {
            if meshlet.triangles.is_empty() {
                continue;
            }
            let bounds = compute_meshlet_bounds(meshlet, adapter);
            let center = bounds.center;
            let radius = bounds.radius.max(0.0);
            let lod = lod.unwrap_or(Sphere { center, radius });

            let index_offset = self.indices.len() as u32;
            for &slot in meshlet.triangles {
                self.indices.push(meshlet.vertices[slot as usize]);
            }
            ids.push(self.clusters.len() as u32);
            self.clusters.push(DagCluster {
                index_offset,
                index_count: meshlet.triangles.len() as u32,
                center,
                radius,
                lod_center: lod.center,
                lod_radius: lod.radius,
                error,
                parent_lod_center: lod.center,
                parent_lod_radius: lod.radius,
                parent_error: f32::INFINITY,
                level,
                group,
            });
        }
        ids
    }

    /// Triangle index slice of one cluster.
    fn slice_of(&self, cluster: u32) -> &[u32] {
        let c = &self.clusters[cluster as usize];
        let start = c.index_offset as usize;
        &self.indices[start..start + c.index_count as usize]
    }

    /// Undirected edges of one cluster (canonical vertex ids).
    fn cluster_edges(&self, cluster: u32) -> Vec<(u32, u32)> {
        let mut edges = Vec::new();
        for tri in self.slice_of(cluster).chunks_exact(3) {
            edges.push((tri[0], tri[1]));
            edges.push((tri[1], tri[2]));
            edges.push((tri[2], tri[0]));
        }
        edges
    }

    /// Bounding sphere enclosing the LOD spheres of the members.
    fn enclosing_sphere(&self, members: &[u32]) -> Sphere {
        let spheres: Vec<Sphere> = members
            .iter()
            .map(|&m| {
                let c = &self.clusters[m as usize];
                Sphere {
                    center: c.lod_center,
                    radius: c.lod_radius,
                }
            })
            .collect();
        let n = spheres.len().max(1) as f32;
        let mut center = [0.0f32; 3];
        for s in &spheres {
            for (k, value) in center.iter_mut().enumerate() {
                *value += s.center[k] / n;
            }
        }
        let radius = spheres
            .iter()
            .map(|s| dist(s.center, center) + s.radius)
            .fold(0.0f32, f32::max);
        // Small relative margin so float rounding cannot leave a child sphere outside.
        Sphere {
            center,
            radius: radius * (1.0 + SPHERE_MARGIN_REL),
        }
    }

    /// Simplifies the union of the members with a frozen outer border.
    /// Returns the simplified indices (canonical ids) and the group error, or `None` when the group is stuck.
    fn simplify_group(
        &self,
        adapter: &VertexDataAdapter<'_>,
        members: &[u32],
        ratio: f32,
    ) -> Option<(Vec<u32>, f32)> {
        let mut flat: Vec<u32> = Vec::new();
        for &m in members {
            flat.extend_from_slice(self.slice_of(m));
        }
        let target = (((flat.len() as f32) * ratio) as usize / 3 * 3).max(3);
        let mut err = 0.0f32;
        let raw = simplify(
            &flat,
            adapter,
            target,
            GROUP_TARGET_ERROR,
            SimplifyOptions::LockBorder,
            Some(&mut err),
        );
        if raw.len() < 3 || raw.len() as f32 >= flat.len() as f32 * STUCK_SHARE {
            return None;
        }
        let e = err * simplify_scale(adapter);
        if !e.is_finite() {
            return None;
        }
        let child_max = members
            .iter()
            .map(|&m| self.clusters[m as usize].error)
            .fold(0.0f32, f32::max);
        let error = e.max(child_max) + child_max * ERROR_MARGIN_REL + ERROR_MARGIN_ABS;
        let simplified = raw.iter().map(|&i| self.weld[i as usize]).collect();
        Some((simplified, error))
    }

    /// Records the group that consumed `members` as their parent.
    fn consume(&mut self, members: &[u32], sphere: Sphere, error: f32) {
        for &m in members {
            let c = &mut self.clusters[m as usize];
            c.parent_lod_center = sphere.center;
            c.parent_lod_radius = sphere.radius;
            c.parent_error = error;
        }
    }
}

/// Builds the hierarchy for a mesh. Positions are welded internally by position (same position = same vertex for adjacency and simplification);
/// the returned `indices` are indices into the ORIGINAL vertex array of the mesh, so attributes stay intact.
///
/// Level 0 are meshlets of the welded mesh (error 0). Each level groups neighbouring clusters
/// (`params.group_size`), simplifies every group to `params.target_ratio` of its indices with the
/// outer border locked, and cuts the result into new clusters. A group that does not shrink
/// makes its members roots. Degenerate triangles (two corners welding to one vertex) are dropped.
pub fn build_cluster_dag(
    vertices: &[Vertex],
    indices: &[u32],
    params: &DagParams,
) -> Result<(ClusterDag, Vec<u32>), DagError> {
    if let Some(bad) = indices.iter().find(|&&i| i as usize >= vertices.len()) {
        return Err(DagError::Invalid(vec![format!(
            "index {bad} is out of range"
        )]));
    }
    let weld = weld_positions(vertices);
    let mut tris: Vec<u32> = Vec::with_capacity(indices.len());
    for t in indices.chunks_exact(3) {
        let w = [
            weld[t[0] as usize],
            weld[t[1] as usize],
            weld[t[2] as usize],
        ];
        if w[0] != w[1] && w[1] != w[2] && w[0] != w[2] {
            tris.extend_from_slice(&w);
        }
    }
    if tris.is_empty() {
        return Err(DagError::Empty);
    }

    let adapter = position_adapter(vertices);
    let mut b = DagBuilder {
        weld,
        clusters: Vec::new(),
        indices: Vec::new(),
    };
    let mut cur = b.emit(&adapter, &tris, 0, NO_GROUP, None, 0.0);
    let mut group_count: u32 = 0;
    let mut level: u32 = 0;
    while cur.len() > 1 && level < params.max_levels {
        let edges: Vec<Vec<(u32, u32)>> = cur.iter().map(|&c| b.cluster_edges(c)).collect();
        let shared = adjacency(&edges);
        let groups = group_clusters(cur.len(), &shared, params.group_size);

        let mut next = Vec::new();
        for local in groups {
            let members: Vec<u32> = local.iter().map(|&i| cur[i as usize]).collect();
            let Some((simplified, error)) =
                b.simplify_group(&adapter, &members, params.target_ratio)
            else {
                continue; // stuck: members stay roots
            };
            let sphere = b.enclosing_sphere(&members);
            b.consume(&members, sphere, error);
            let gid = group_count;
            group_count += 1;
            next.extend(b.emit(&adapter, &simplified, level + 1, gid, Some(sphere), error));
        }
        cur = next;
        level += 1;
    }

    let mut dag = ClusterDag {
        clusters: b.clusters,
        root_count: 0,
    };
    dag.root_count = dag
        .clusters
        .iter()
        .filter(|c| c.parent_error == f32::INFINITY)
        .count() as u32;
    let problems = validate(&dag);
    if !problems.is_empty() {
        return Err(DagError::Invalid(problems));
    }
    Ok((dag, b.indices))
}
