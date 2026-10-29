//! Cluster hierarchy (DAG) for virtual geometry.
//!
//! Leaves are meshlets of the source mesh (error 0). Every group of
//! neighbouring clusters is simplified with the outer border frozen and the
//! result is cut into new clusters that carry the group's error and bounding
//! sphere. A cluster is drawn when its own error is precise enough on screen
//! and its parent's error is not.
//!
//! Convention: `error`/`lod_*` of a cluster are those of the group that
//! CREATED it; `parent_*` are those of the group that CONSUMED it. Leaves have
//! error 0 and `group == u32::MAX`. Clusters that are never consumed (roots)
//! keep `parent_error == f32::INFINITY`.

mod asset;
mod build;
mod gpu;
mod group;

pub use asset::{VGDagAsset, build_vg_dag_asset};
pub use build::{build_cluster_dag, weld_positions};
pub use gpu::{DagClusterGpu, DagInstance, gpu_clusters, select_for_instances};
pub use group::{adjacency, group_clusters};

/// One cluster of the hierarchy. Triangles live in the asset's index buffer at `index_offset..index_offset+index_count`.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DagCluster {
    /// First index of this cluster's triangles in the index buffer.
    pub index_offset: u32,
    /// Number of indices (three per triangle).
    pub index_count: u32,
    /// Bounding sphere of this cluster's geometry (local space).
    pub center: [f32; 3],
    /// Radius of the geometry bounding sphere.
    pub radius: f32,
    /// Sphere and error that decide when THIS cluster is precise enough. Shared by all clusters of one group.
    pub lod_center: [f32; 3],
    /// Radius of the LOD sphere.
    pub lod_radius: f32,
    /// Error of the group that created this cluster (object units).
    pub error: f32,
    /// Sphere and error of the group that REPLACES this cluster; `f32::INFINITY` error for the root.
    pub parent_lod_center: [f32; 3],
    /// Radius of the parent's LOD sphere.
    pub parent_lod_radius: f32,
    /// Error of the group that consumed this cluster (`f32::INFINITY` for roots).
    pub parent_error: f32,
    /// Level in the hierarchy (0 = full detail leaves).
    pub level: u32,
    /// Group this cluster was created by (u32::MAX for leaves).
    pub group: u32,
}

/// A complete cluster hierarchy plus the list of root clusters.
#[derive(Clone, Debug, Default)]
pub struct ClusterDag {
    /// All clusters, leaves first, then one block per built level.
    pub clusters: Vec<DagCluster>,
    /// Number of clusters without a parent (`parent_error == INFINITY`).
    pub root_count: u32,
}

/// Parameters of the hierarchy build.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DagParams {
    /// Target number of clusters per group (2..=6).
    pub group_size: usize,
    /// Fraction of indices kept when a group is simplified.
    pub target_ratio: f32,
    /// Maximum number of levels built above the leaves.
    pub max_levels: u32,
}

impl Default for DagParams {
    fn default() -> Self {
        Self {
            group_size: 4,
            target_ratio: 0.5,
            max_levels: 24,
        }
    }
}

/// Errors returned by the DAG build.
#[derive(Debug, thiserror::Error)]
pub enum DagError {
    /// The mesh has no usable (non-degenerate) triangles.
    #[error("mesh has no usable triangles")]
    Empty,
    /// The built hierarchy failed validation, or the input is malformed.
    #[error("invalid cluster DAG: {0:?}")]
    Invalid(Vec<String>),
}

/// Screen-space size in pixels of an object-space `error` measured on a sphere `(center, radius)` seen from `eye`
/// with the projection scale `pixels_per_unit_at_1m` (= screen_height / (2 * tan(fov_y/2))). The distance is taken to the nearest point of the sphere (>= near).
pub fn projected_error_px(
    error: f32,
    center: [f32; 3],
    radius: f32,
    eye: [f32; 3],
    pixels_per_unit_at_1m: f32,
) -> f32 {
    let dx = center[0] - eye[0];
    let dy = center[1] - eye[1];
    let dz = center[2] - eye[2];
    let dist = (dx * dx + dy * dy + dz * dz).sqrt();
    let d = (dist - radius).max(0.001);
    error * pixels_per_unit_at_1m / d
}

/// CPU reference of the cut: the clusters to draw for threshold `max_error_px`. A cluster is chosen when
/// projected(error) <= max_error_px && projected(parent_error) > max_error_px.
pub fn cut(
    dag: &ClusterDag,
    eye: [f32; 3],
    pixels_per_unit_at_1m: f32,
    max_error_px: f32,
) -> Vec<u32> {
    dag.clusters
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            let own = projected_error_px(
                c.error,
                c.lod_center,
                c.lod_radius,
                eye,
                pixels_per_unit_at_1m,
            );
            let parent = projected_error_px(
                c.parent_error,
                c.parent_lod_center,
                c.parent_lod_radius,
                eye,
                pixels_per_unit_at_1m,
            );
            own <= max_error_px && parent > max_error_px
        })
        .map(|(i, _)| i as u32)
        .collect()
}

/// Tolerance used by the containment check of parent LOD spheres.
const CONTAINMENT_EPS: f32 = 1e-4;

/// Euclidean distance between two points.
pub(super) fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

/// Structural checks used by tests and by the cook: returns the list of violated rules (empty = ok).
pub fn validate(dag: &ClusterDag) -> Vec<String> {
    let mut problems = Vec::new();
    let mut roots = 0u32;
    for (i, c) in dag.clusters.iter().enumerate() {
        if c.parent_error == f32::INFINITY {
            roots += 1;
        }
        if c.parent_error < c.error {
            problems.push(format!(
                "cluster {i}: parent error {} is lower than its error {}",
                c.parent_error, c.error
            ));
        }
        if c.group == u32::MAX && c.error != 0.0 {
            problems.push(format!("cluster {i}: leaf has error {}", c.error));
        }
        if c.parent_lod_radius < c.lod_radius {
            problems.push(format!(
                "cluster {i}: parent LOD radius is smaller than its own"
            ));
        }
        if dist(c.lod_center, c.parent_lod_center) + c.lod_radius
            > c.parent_lod_radius + CONTAINMENT_EPS
        {
            problems.push(format!(
                "cluster {i}: parent LOD sphere does not contain its LOD sphere"
            ));
        }
    }
    if roots != dag.root_count {
        problems.push(format!(
            "root_count {} does not match the {roots} clusters without parent",
            dag.root_count
        ));
    }
    let mut first_of_group: std::collections::HashMap<u32, usize> =
        std::collections::HashMap::new();
    for (i, c) in dag.clusters.iter().enumerate() {
        if c.group == u32::MAX {
            continue;
        }
        match first_of_group.get(&c.group) {
            None => {
                first_of_group.insert(c.group, i);
            }
            Some(&first) => {
                let f = &dag.clusters[first];
                if f.lod_center != c.lod_center
                    || f.lod_radius != c.lod_radius
                    || f.error != c.error
                {
                    problems.push(format!(
                        "clusters {first} and {i} share group {} but differ in LOD data",
                        c.group
                    ));
                }
            }
        }
    }
    problems
}

/// Number of undirected edges that occur exactly once among the given triangles, with vertices
/// identified through `weld` (`weld[v]` = canonical vertex). A closed surface gives 0.
pub fn open_edge_count(triangles: impl Iterator<Item = (u32, u32, u32)>, weld: &[u32]) -> usize {
    let canon = |v: u32| weld.get(v as usize).copied().unwrap_or(v);
    let mut counts: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
    for (a, b, c) in triangles {
        let v = [canon(a), canon(b), canon(c)];
        for (x, y) in [(v[0], v[1]), (v[1], v[2]), (v[2], v[0])] {
            if x == y {
                continue;
            }
            *counts.entry((x.min(y), x.max(y))).or_insert(0) += 1;
        }
    }
    counts.values().filter(|&&n| n == 1).count()
}
