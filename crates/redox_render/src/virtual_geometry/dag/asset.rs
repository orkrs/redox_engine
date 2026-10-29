//! DAG virtual geometry asset: the mesh vertices, the cluster index buffer and the hierarchy.

use super::{ClusterDag, DagError, DagParams, build_cluster_dag};
use crate::mesh::{Mesh, Vertex};
use crate::virtual_geometry::asset_pipeline::compute_aabb;

/// Hierarchical virtual geometry asset. Unlike the flat `VGAssetData`, its clusters are
/// selected per screen error through [`ClusterDag`] (see `dag::cut`).
pub struct VGDagAsset {
    /// Original mesh vertices; every index refers to this array.
    pub vertices: Vec<Vertex>,
    /// Triangle index buffer shared by all clusters (see `DagCluster::index_offset`).
    pub indices: Vec<u32>,
    /// Cluster hierarchy.
    pub dag: ClusterDag,
    /// Local-space AABB `[min, max]`.
    pub aabb: [[f32; 3]; 2],
    /// Material index of the whole asset.
    pub material_index: u32,
}

/// Builds a DAG asset from a standard mesh.
pub fn build_vg_dag_asset(
    mesh: &Mesh,
    params: &DagParams,
    material_index: u32,
) -> Result<VGDagAsset, DagError> {
    let (dag, indices) = build_cluster_dag(&mesh.vertices, &mesh.indices, params)?;
    Ok(VGDagAsset {
        vertices: mesh.vertices.clone(),
        indices,
        dag,
        aabb: compute_aabb(&mesh.vertices),
        material_index,
    })
}
