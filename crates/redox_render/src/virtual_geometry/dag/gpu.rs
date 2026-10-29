//! GPU layout of the cluster hierarchy, children index and CPU reference for instanced selection.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};

use super::{ClusterDag, DagCluster, cut};

/// Sphere and error of a group, as bit patterns, used to match a group with the clusters it consumed.
fn group_key(center: [f32; 3], radius: f32, error: f32) -> [u32; 5] {
    [
        center[0].to_bits(),
        center[1].to_bits(),
        center[2].to_bits(),
        radius.to_bits(),
        error.to_bits(),
    ]
}

impl ClusterDag {
    /// Children index of the hierarchy.
    ///
    /// Convention: the children of group `g` are the clusters that `g` consumed (the clusters
    /// that were simplified into the clusters created by `g`). Every cluster of a group has the
    /// same children. Returns `(child_ids, per_group)`: `per_group[g] = (first, count)` is the
    /// range of `child_ids` holding the children of group `g`; groups without children get `(0, 0)`.
    /// Leaves and roots that were never consumed have no group entry above them.
    pub fn children_index(&self) -> (Vec<u32>, Vec<(u32, u32)>) {
        let group_count = self
            .clusters
            .iter()
            .filter(|c| c.group != u32::MAX)
            .map(|c| c.group as usize + 1)
            .max()
            .unwrap_or(0);

        // The sphere and error a group was built with are stored in every cluster it created.
        let mut group_by_key: HashMap<[u32; 5], u32> = HashMap::new();
        for c in self.clusters.iter().filter(|c| c.group != u32::MAX) {
            group_by_key.insert(group_key(c.lod_center, c.lod_radius, c.error), c.group);
        }

        let mut lists: Vec<Vec<u32>> = vec![Vec::new(); group_count];
        for (i, c) in self.clusters.iter().enumerate() {
            if c.parent_error == f32::INFINITY {
                continue;
            }
            let key = group_key(c.parent_lod_center, c.parent_lod_radius, c.parent_error);
            if let Some(&g) = group_by_key.get(&key) {
                lists[g as usize].push(i as u32);
            }
        }

        let mut child_ids = Vec::new();
        let mut ranges = Vec::with_capacity(group_count);
        for list in &lists {
            ranges.push((child_ids.len() as u32, list.len() as u32));
            child_ids.extend_from_slice(list);
        }
        (child_ids, ranges)
    }
}

/// GPU mirror of [`DagCluster`] plus its children range. 96 bytes: 80 bytes of fields and a 16-byte pad.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct DagClusterGpu {
    /// See [`DagCluster::index_offset`].
    pub index_offset: u32,
    /// See [`DagCluster::index_count`].
    pub index_count: u32,
    /// See [`DagCluster::center`].
    pub center: [f32; 3],
    /// See [`DagCluster::radius`].
    pub radius: f32,
    /// See [`DagCluster::lod_center`].
    pub lod_center: [f32; 3],
    /// See [`DagCluster::lod_radius`].
    pub lod_radius: f32,
    /// See [`DagCluster::error`].
    pub error: f32,
    /// See [`DagCluster::parent_lod_center`].
    pub parent_lod_center: [f32; 3],
    /// See [`DagCluster::parent_lod_radius`].
    pub parent_lod_radius: f32,
    /// See [`DagCluster::parent_error`].
    pub parent_error: f32,
    /// See [`DagCluster::level`].
    pub level: u32,
    /// See [`DagCluster::group`].
    pub group: u32,
    /// First entry of the children range in the child index buffer.
    pub first_child: u32,
    /// Number of children (0 for leaves).
    pub child_count: u32,
    /// Padding to 96 bytes.
    pub _pad: [u32; 4],
}

impl DagClusterGpu {
    /// Converts a cluster and its children range.
    pub fn from_cluster(c: &DagCluster, first_child: u32, child_count: u32) -> Self {
        Self {
            index_offset: c.index_offset,
            index_count: c.index_count,
            center: c.center,
            radius: c.radius,
            lod_center: c.lod_center,
            lod_radius: c.lod_radius,
            error: c.error,
            parent_lod_center: c.parent_lod_center,
            parent_lod_radius: c.parent_lod_radius,
            parent_error: c.parent_error,
            level: c.level,
            group: c.group,
            first_child,
            child_count,
            _pad: [0; 4],
        }
    }
}

const _: () = assert!(std::mem::size_of::<DagClusterGpu>() == 96);
const _: () = assert!(std::mem::size_of::<DagClusterGpu>().is_multiple_of(16));

/// GPU-side cluster records of a hierarchy, in the same order as `dag.clusters`.
pub fn gpu_clusters(dag: &ClusterDag) -> (Vec<DagClusterGpu>, Vec<u32>) {
    let (child_ids, ranges) = dag.children_index();
    let gpu = dag
        .clusters
        .iter()
        .map(|c| {
            let (first, count) = if c.group == u32::MAX {
                (0, 0)
            } else {
                ranges.get(c.group as usize).copied().unwrap_or((0, 0))
            };
            DagClusterGpu::from_cluster(c, first, count)
        })
        .collect();
    (gpu, child_ids)
}

/// One placed copy of a DAG asset.
#[derive(Clone, Copy, Debug)]
pub struct DagInstance {
    /// Index of the hierarchy in the slice given to [`select_for_instances`].
    pub dag: usize,
    /// Local-to-world affine transform, column-major (`transform[column][row]`). Uniform scale is assumed.
    pub transform: [[f32; 4]; 4],
}

/// CPU reference of GPU instance selection: for every instance, the clusters chosen by [`cut`]
/// with the eye moved into the instance's local space. Returns `(instance, cluster)` pairs.
pub fn select_for_instances(
    dags: &[ClusterDag],
    instances: &[DagInstance],
    eye: [f32; 3],
    pixels_per_unit_at_1m: f32,
    max_error_px: f32,
) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for (ii, inst) in instances.iter().enumerate() {
        let Some(dag) = dags.get(inst.dag) else {
            continue;
        };
        let Some(local_eye) = world_to_local(inst.transform, eye) else {
            continue;
        };
        for c in cut(dag, local_eye, pixels_per_unit_at_1m, max_error_px) {
            out.push((ii as u32, c));
        }
    }
    out
}

/// Maps a world point into the local space of an affine transform (`None` if singular).
fn world_to_local(t: [[f32; 4]; 4], p: [f32; 3]) -> Option<[f32; 3]> {
    let m = [
        [t[0][0], t[1][0], t[2][0]],
        [t[0][1], t[1][1], t[2][1]],
        [t[0][2], t[1][2], t[2][2]],
    ];
    let b = [p[0] - t[3][0], p[1] - t[3][1], p[2] - t[3][2]];
    solve_3x3(m, b)
}

fn det3(m: [[f32; 3]; 3]) -> f32 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

/// Solves `m * x = b` with Cramer's rule.
fn solve_3x3(m: [[f32; 3]; 3], b: [f32; 3]) -> Option<[f32; 3]> {
    let det = det3(m);
    if det.abs() < 1e-12 {
        return None;
    }
    let mut x = [0.0f32; 3];
    for (col, value) in x.iter_mut().enumerate() {
        let mut mc = m;
        for (row, r) in mc.iter_mut().enumerate() {
            r[col] = b[row];
        }
        *value = det3(mc) / det;
    }
    Some(x)
}
