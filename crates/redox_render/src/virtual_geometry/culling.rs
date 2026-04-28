//! CPU-side frustum culling for virtual geometry.
//!
//! For each registered instance, every meshlet is tested against the camera
//! frustum using its bounding sphere.  Visible meshlets generate
//! [`DrawIndexedIndirectCmd`] entries that are uploaded to the indirect draw
//! buffer.
//!
//! ## Future work
//!
//! A GPU compute shader can replace this pass for scenes with millions of
//! meshlets.  The interface (`CullingResult`) remains the same.

use super::meshlet::{DrawIndexedIndirectCmd, MeshletDescriptor, VGInstanceData};

// ── Frustum ──────────────────────────────────────────────────────────────────

/// Six frustum planes in the form `(nx, ny, nz, d)`.
///
/// A point `p` is inside the frustum when `dot(n, p) + d >= 0` for all planes.
pub struct Frustum {
    /// [left, right, bottom, top, near, far]
    pub planes: [[f32; 4]; 6],
}

impl Frustum {
    /// Extract frustum planes from a column-major view-projection matrix.
    ///
    /// Uses the Gribb–Hartmann method; planes are normalised.
    pub fn from_view_proj(vp: &[[f32; 4]; 4]) -> Self {
        // Interpret vp as col-major: vp[col][row]
        let m = |col: usize, row: usize| vp[col][row];

        let row = |r: usize| [m(0, r), m(1, r), m(2, r), m(3, r)];

        let r0 = row(0);
        let r1 = row(1);
        let r2 = row(2);
        let r3 = row(3);

        let left   = add4(r3, r0);
        let right  = sub4(r3, r0);
        let bottom = add4(r3, r1);
        let top    = sub4(r3, r1);
        let near   = add4(r3, r2);
        let far    = sub4(r3, r2);

        let planes = [left, right, bottom, top, near, far].map(normalize_plane);
        Frustum { planes }
    }

    /// Returns `true` if the sphere may be visible (passes the frustum test).
    #[inline]
    pub fn test_sphere(&self, center: [f32; 3], radius: f32) -> bool {
        for p in &self.planes {
            let d = p[0] * center[0] + p[1] * center[1] + p[2] * center[2] + p[3];
            if d < -radius {
                return false;
            }
        }
        true
    }
}

#[inline]
fn add4(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}

#[inline]
fn sub4(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]]
}

fn normalize_plane(p: [f32; 4]) -> [f32; 4] {
    let len = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
    if len < 1e-8 {
        p
    } else {
        [p[0] / len, p[1] / len, p[2] / len, p[3] / len]
    }
}

// ── Transform helpers ─────────────────────────────────────────────────────────

/// Transform a point by a column-major 4×4 matrix.
fn transform_point(m: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    let w = m[0][3] * p[0] + m[1][3] * p[1] + m[2][3] * p[2] + m[3][3];
    let inv_w = if w.abs() > 1e-6 { 1.0 / w } else { 1.0 };
    [
        (m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0]) * inv_w,
        (m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1]) * inv_w,
        (m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2]) * inv_w,
    ]
}

/// Approximate scale factor (max column length) of a 4×4 matrix.
fn max_scale(m: &[[f32; 4]; 4]) -> f32 {
    let mut max_sq = 0.0f32;
    for col in m.iter().take(3) {
        let sq = col[0] * col[0] + col[1] * col[1] + col[2] * col[2];
        max_sq = max_sq.max(sq);
    }
    max_sq.sqrt()
}

// ── Culling result ────────────────────────────────────────────────────────────

/// Output of the culling pass: a compact list of visible draw commands.
pub struct CullingResult {
    pub commands: Vec<DrawIndexedIndirectCmd>,
    pub visible_meshlet_count: u32,
    pub visible_triangle_count: u64,
    pub total_meshlet_count: u32,
}

// ── Main culling function ─────────────────────────────────────────────────────

/// Cull all meshlets across all instances against the given frustum.
///
/// `global_meshlets` is the flat GPU meshlet array (all assets merged).
/// `instances` is the list of active instances.
pub fn cull_meshlets(
    instances: &[VGInstanceData],
    global_meshlets: &[MeshletDescriptor],
    frustum: &Frustum,
) -> CullingResult {
    let mut commands = Vec::new();
    let mut visible_tris = 0u64;
    let total = global_meshlets.len() as u32;

    for (inst_idx, inst) in instances.iter().enumerate() {
        let start = inst.meshlet_offset as usize;
        let end = (inst.meshlet_offset + inst.meshlet_count) as usize;

        if start > global_meshlets.len() {
            continue;
        }
        let end = end.min(global_meshlets.len());

        let scale = max_scale(&inst.transform);

        for meshlet in &global_meshlets[start..end] {
            let world_center = transform_point(&inst.transform, meshlet.sphere_center);
            let world_radius = meshlet.sphere_radius * scale;

            if !frustum.test_sphere(world_center, world_radius) {
                continue;
            }

            commands.push(DrawIndexedIndirectCmd {
                index_count: meshlet.index_count,
                instance_count: 1,
                first_index: meshlet.index_offset,
                base_vertex: 0,
                first_instance: inst_idx as u32,
            });
            visible_tris += (meshlet.index_count / 3) as u64;
        }
    }

    CullingResult {
        visible_meshlet_count: commands.len() as u32,
        visible_triangle_count: visible_tris,
        total_meshlet_count: total,
        commands,
    }
}
