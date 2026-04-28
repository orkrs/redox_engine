//! Core data structures for virtual geometry / meshlet rendering.
//!
//! A **meshlet** (or *cluster*) is a small group of triangles (≤ 124) and their
//! shared vertices (≤ 64) carved out of a larger mesh.  Splitting a mesh into
//! meshlets enables:
//!
//! - Per-cluster GPU frustum / occlusion culling.
//! - Automatic LOD selection per cluster.
//! - Efficient indirect rendering with `multi_draw_indexed_indirect`.

use bytemuck::{Pod, Zeroable};
use crate::mesh::Vertex;

// ── Constants ────────────────────────────────────────────────────────────────

/// Maximum number of vertices per meshlet.
pub const MAX_VERTICES_PER_MESHLET: usize = 64;

/// Maximum number of triangles per meshlet (index triples).
pub const MAX_TRIANGLES_PER_MESHLET: usize = 124;

/// Maximum meshlets that can be stored in the global GPU buffer.
pub const MAX_GLOBAL_MESHLETS: usize = 1 << 20; // 1M meshlets

/// Maximum instances that can be registered simultaneously.
pub const MAX_VG_INSTANCES: usize = 4096;

/// Maximum visible draw commands per frame.
pub const MAX_VISIBLE_COMMANDS: usize = 1 << 17; // 128K draws

// ── GPU-uploadable structures ─────────────────────────────────────────────────

/// Descriptor for a single meshlet stored on the GPU.
///
/// All offsets are absolute into the **global** vertex / index buffers owned by
/// [`super::runtime::VGSystem`].
///
/// Total size: 64 bytes (4 × 16-byte aligned rows).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct MeshletDescriptor {
    /// First vertex in the global vertex buffer.
    pub vertex_offset: u32,
    /// Number of vertices belonging to this meshlet.
    pub vertex_count: u32,
    /// First index in the global index buffer.
    pub index_offset: u32,
    /// Number of indices (= `triangle_count * 3`).
    pub index_count: u32,

    /// Bounding-sphere centre in **local** (object) space.
    pub sphere_center: [f32; 3],
    /// Bounding-sphere radius.
    pub sphere_radius: f32,

    /// Backface-culling cone axis (average of face normals).
    pub cone_axis: [f32; 3],
    /// `cos(half_angle)` of the backface cone.  Cull if
    /// `dot(cone_axis, to_camera) < cone_cutoff`.
    pub cone_cutoff: f32,

    /// Geometric error at this LOD level (world-space units).
    pub lod_error: f32,
    /// LOD level index (0 = most detailed).
    pub lod_level: u32,
    /// Index into the global material array.
    pub material_index: u32,
    pub _pad: u32,
}

/// Per-instance data that lives in a GPU storage buffer.
///
/// Written from the CPU once per instance spawn / transform change.
///
/// Total size: 80 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct VGInstanceData {
    /// Column-major 4×4 world transform (matches `wgpu` / WGSL `mat4x4<f32>`).
    pub transform: [[f32; 4]; 4],
    /// Index of the first meshlet for this instance in the global meshlet array.
    pub meshlet_offset: u32,
    /// Total number of meshlets belonging to this instance.
    pub meshlet_count: u32,
    /// Bitfield flags (reserved for future use).
    pub flags: u32,
    pub _pad: u32,
}

/// One entry in the indirect draw command buffer, matching the layout expected
/// by `wgpu::RenderPass::draw_indexed_indirect`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct DrawIndexedIndirectCmd {
    pub index_count: u32,
    pub instance_count: u32,
    pub first_index: u32,
    pub base_vertex: i32,
    /// Instance index; used by the vertex shader to fetch the transform from
    /// the `vg_instances` storage buffer.
    pub first_instance: u32,
}

// ── CPU-side asset data ───────────────────────────────────────────────────────

/// CPU-side virtual geometry asset produced by the asset pipeline.
///
/// This is uploaded once to the global GPU buffers inside [`super::runtime::VGSystem`].
/// After upload the CPU-side vectors may be dropped.
pub struct VGAssetData {
    /// All vertices (may include duplicates shared between different meshlets).
    pub vertices: Vec<Vertex>,
    /// Absolute indices into `vertices`.  Pre-remapped so that every three
    /// consecutive entries form one triangle.
    pub indices: Vec<u32>,
    /// Per-meshlet descriptors; offsets are relative to **this asset's own**
    /// vertex/index slices.  They are rebased when the data is uploaded to
    /// the global buffers.
    pub meshlets: Vec<MeshletDescriptor>,
    /// Axis-aligned bounding box in local space: `[min, max]`.
    pub aabb: [[f32; 3]; 2],
}

impl VGAssetData {
    /// Total number of triangles across all meshlets.
    pub fn triangle_count(&self) -> u64 {
        self.meshlets.iter().map(|m| (m.index_count / 3) as u64).sum()
    }
}
