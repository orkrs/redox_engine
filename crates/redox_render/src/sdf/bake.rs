//! CPU baking of 64³ signed distance fields from triangle meshes.
//!
//! # Algorithm
//!
//! 1. All triangles are extracted into a flat array and wrapped in a
//!    median-split Bounding Volume Hierarchy (longest-axis split, leaf
//!    capacity 4, flat node pool).
//! 2. The field domain is the axis-aligned cube of side `world_size`
//!    centred on the mesh AABB centre. Texel `(x, y, z)` stores the signed
//!    distance at the texel centre
//!    `centre + world_size * ((x,y,z) + 0.5) / 64 - world_size / 2`,
//!    i.e. texel centres sit at half-texel offsets from the domain faces.
//! 3. Unsigned distance: exact closest-point-on-mesh query through the BVH
//!    (best-first branch-and-bound over node AABBs, Ericson's
//!    point-in-closest-triangle test per visited leaf).
//! 4. Sign: negative inside the solid, positive outside (the standard SDF
//!    convention), stored in meters.
//!
//! ## Sign method — ray parity (chosen deliberately)
//!
//! The plan allowed either ray parity or the triangle-winding heuristic
//! (`dot(triangle_normal, texel - closest_point)`). We use **ray parity**:
//! a ray is cast from the texel centre in a fixed, non-axis-aligned
//! direction and mesh intersections are counted via the BVH; an odd count
//! means the point is inside. The winding heuristic was rejected because it
//! systematically mis-signs points whose nearest surface feature is an edge
//! or vertex shared by faces with different normals — every convex corner
//! of a cube, for example, where the closest triangle's face normal points
//! sideways while the texel sits outside along the diagonal. Parity only
//! assumes a watertight, consistently wound mesh, which is the same
//! assumption every downstream consumer (shadows, DDGI) makes. Points
//! outside the mesh AABB skip the ray cast entirely (they are provably
//! outside the solid).
//!
//! Baked fields are pure CPU data; packing them into the GPU 128³ brick
//! atlas happens in a later phase (see [`crate::sdf::atlas`]).

use crate::mesh::Mesh;
use redox_math::Vec3;

/// Field resolution along each axis (64³ texels per brick).
pub const SDF_RESOLUTION: usize = 64;

/// A baked 64³ signed distance field covering a cubic world-space domain.
///
/// The domain is centred on the baked mesh's AABB centre; callers that need
/// absolute world placement wrap the field in an
/// [`crate::sdf::SdfBrick`], which carries `origin_ws` (the world-space
/// centre of the domain) alongside the field.
#[derive(Clone, Debug)]
pub struct DistanceField64 {
    /// Signed distance in meters at each texel centre.
    ///
    /// Indexing: `i = (z * SDF_RESOLUTION + y) * SDF_RESOLUTION + x`
    /// (x fastest). Negative = inside the solid.
    pub data: Vec<f32>,
    /// Side length of the cubic (recommended) domain in meters.
    pub world_size: Vec3,
}

impl DistanceField64 {
    /// Linear index into [`Self::data`] for texel `(x, y, z)`.
    #[inline]
    pub fn index(x: usize, y: usize, z: usize) -> usize {
        (z * SDF_RESOLUTION + y) * SDF_RESOLUTION + x
    }

    /// Size of one texel edge in meters per axis.
    #[inline]
    pub fn texel_world_size(&self) -> Vec3 {
        self.world_size / SDF_RESOLUTION as f32
    }

    /// Trilinearly samples the field at domain-local coordinates
    /// (`p` relative to the domain centre, in meters; the domain spans
    /// `±world_size / 2`). Positions outside the domain clamp to the
    /// border texels.
    pub fn sample(&self, local: Vec3) -> f32 {
        // Continuous grid coordinate in [-0.5 * (RES-1), 0.5 * (RES-1)].
        let g = (local / self.world_size + Vec3::splat(0.5)) * Vec3::splat(SDF_RESOLUTION as f32)
            - Vec3::splat(0.5);
        let lo = g.floor();
        let fr = g - lo;
        let clampi =
            |v: f32| -> usize { v.round().clamp(0.0, (SDF_RESOLUTION - 1) as f32) as usize };
        let x0 = clampi(lo.x);
        let y0 = clampi(lo.y);
        let z0 = clampi(lo.z);
        let x1 = (x0 + 1).min(SDF_RESOLUTION - 1);
        let y1 = (y0 + 1).min(SDF_RESOLUTION - 1);
        let z1 = (z0 + 1).min(SDF_RESOLUTION - 1);
        let (fx, fy, fz) = (fr.x, fr.y, fr.z);
        let d = &self.data;
        let c00 = d[Self::index(x0, y0, z0)].mul_add(1.0 - fx, d[Self::index(x1, y0, z0)] * fx);
        let c10 = d[Self::index(x0, y1, z0)].mul_add(1.0 - fx, d[Self::index(x1, y1, z0)] * fx);
        let c01 = d[Self::index(x0, y0, z1)].mul_add(1.0 - fx, d[Self::index(x1, y0, z1)] * fx);
        let c11 = d[Self::index(x0, y1, z1)].mul_add(1.0 - fx, d[Self::index(x1, y1, z1)] * fx);
        let c0 = c00.mul_add(1.0 - fy, c10 * fy);
        let c1 = c01.mul_add(1.0 - fy, c11 * fy);
        c0.mul_add(1.0 - fz, c1 * fz)
    }
}

/// Bakes a 64³ signed distance field for `mesh` over a domain of side
/// `world_size` centred on the mesh AABB centre. An empty mesh yields a
/// uniformly "far outside" field.
pub fn bake_mesh(mesh: &Mesh, world_size: Vec3) -> DistanceField64 {
    let tri_count = mesh.indices.len() / 3;
    debug_assert_eq!(
        mesh.indices.len() % 3,
        0,
        "mesh index buffer must be a multiple of 3"
    );

    if tri_count == 0 {
        return DistanceField64 {
            data: vec![world_size.max_element(); SDF_RESOLUTION.pow(3)],
            world_size,
        };
    }

    let mut tris = Vec::with_capacity(tri_count);
    for tri in mesh.indices.chunks_exact(3) {
        let a = mesh.vertices[tri[0] as usize].position;
        let b = mesh.vertices[tri[1] as usize].position;
        let c = mesh.vertices[tri[2] as usize].position;
        tris.push([
            Vec3::new(a[0], a[1], a[2]),
            Vec3::new(b[0], b[1], b[2]),
            Vec3::new(c[0], c[1], c[2]),
        ]);
    }
    let bvh = TriBvh::new(tris);

    let centre = mesh.aabb.center();
    let res = SDF_RESOLUTION as f32;
    // Points provably outside the (slightly inflated) mesh AABB are outside
    // the solid — skips the parity ray cast for most exterior texels.
    let eps = 1.0e-4;
    let outer_min = mesh.aabb.min - Vec3::splat(eps);
    let outer_max = mesh.aabb.max + Vec3::splat(eps);

    let mut data = vec![0.0f32; SDF_RESOLUTION.pow(3)];
    for z in 0..SDF_RESOLUTION {
        let lz = ((z as f32 + 0.5) / res - 0.5) * world_size.z;
        for y in 0..SDF_RESOLUTION {
            let ly = ((y as f32 + 0.5) / res - 0.5) * world_size.y;
            for x in 0..SDF_RESOLUTION {
                let lx = ((x as f32 + 0.5) / res - 0.5) * world_size.x;
                let local = Vec3::new(lx, ly, lz);
                let p = centre + local;
                let dist = bvh.closest_distance(p);
                let inside = p.x > outer_min.x
                    && p.y > outer_min.y
                    && p.z > outer_min.z
                    && p.x < outer_max.x
                    && p.y < outer_max.y
                    && p.z < outer_max.z;
                let sign = if inside && bvh.parity_inside(p) {
                    -1.0
                } else {
                    1.0
                };
                data[DistanceField64::index(x, y, z)] = sign * dist;
            }
        }
    }

    DistanceField64 { data, world_size }
}

// ---------------------------------------------------------------------------
// Triangle BVH
// ---------------------------------------------------------------------------

const MAX_LEAF_TRIS: usize = 4;
/// Fixed parity ray direction.
///
/// Two requirements. (1) Not axis-aligned, so it never runs parallel to the
/// dominant face planes of typical axis-aligned meshes. (2) The component
/// **ratios must be irrational** — with a rational direction such as
/// `normalize(2, 3, 5)`, rays cast from dyadic-grid texel centres hit mesh
/// edges *exactly* in f32 arithmetic in systematic line families (the
/// preimage planes of each edge intersect the texel lattice in lattice
/// lines), double-counting crossings and flipping the parity sign for those
/// texels. Irrational ratios (φ and √3 here) make exact edge hits
/// measure-zero accidents instead of a systematic failure mode.
const PARITY_DIR: Vec3 = Vec3::new(0.388_738_4, 0.628_942_5, 0.673_264_2); // normalize(1, φ, √3)

struct BvhNode {
    bmin: Vec3,
    bmax: Vec3,
    /// Leaf: index of the first triangle in `tri_order`.
    /// Internal: index of the left child (right child is `left_first + 1`).
    left_first: u32,
    /// Leaf: number of triangles. Internal: 0.
    count: u32,
}

/// Flat median-split BVH over triangles, supporting exact closest-point
/// queries and ray-intersection counting (for the parity sign test).
struct TriBvh {
    nodes: Vec<BvhNode>,
    /// Triangle indices in BVH leaf order.
    tri_order: Vec<u32>,
    /// Flat triangle storage: 3 vertices per triangle.
    tris: Vec<[Vec3; 3]>,
}

impl TriBvh {
    fn new(tris: Vec<[Vec3; 3]>) -> Self {
        let tri_count = tris.len();
        let centroids: Vec<Vec3> = tris.iter().map(|t| (t[0] + t[1] + t[2]) / 3.0).collect();
        let mut bvh = Self {
            nodes: Vec::with_capacity(2 * tri_count),
            tri_order: (0..tri_count as u32).collect(),
            tris,
        };
        if tri_count > 0 {
            bvh.nodes.push(BvhNode {
                bmin: Vec3::ZERO,
                bmax: Vec3::ZERO,
                left_first: 0,
                count: 0,
            });
            bvh.build_into(0, 0, tri_count as u32, &centroids);
        }
        bvh
    }

    /// Recursively fills `slot` with the node covering triangle range
    /// `[start, end)` (indices into `tri_order`).
    ///
    /// The two children of an internal node are allocated **adjacently**
    /// (left at `left_first`, right at `left_first + 1`) before either is
    /// filled, which is the layout the traversal routines rely on.
    fn build_into(&mut self, slot: u32, start: u32, end: u32, centroids: &[Vec3]) {
        let (bmin, bmax) = self.range_bounds(start, end);
        let count = (end - start) as usize;
        self.nodes[slot as usize] = BvhNode {
            bmin,
            bmax,
            left_first: start,
            count: end - start,
        };

        if count <= MAX_LEAF_TRIS {
            return;
        }

        // Median split on the longest axis of the node bounds.
        let extents = bmax - bmin;
        let axis = if extents.x >= extents.y && extents.x >= extents.z {
            0
        } else if extents.y >= extents.z {
            1
        } else {
            2
        };
        let get_c = |t: u32| -> f32 {
            let c = centroids[t as usize];
            [c.x, c.y, c.z][axis]
        };
        self.tri_order[start as usize..end as usize]
            .sort_by(|&a, &b| get_c(a).total_cmp(&get_c(b)));
        let mid = start + (count / 2) as u32;

        let left_idx = self.nodes.len() as u32;
        for _ in 0..2 {
            self.nodes.push(BvhNode {
                bmin: Vec3::ZERO,
                bmax: Vec3::ZERO,
                left_first: 0,
                count: 0,
            });
        }
        self.build_into(left_idx, start, mid, centroids);
        self.build_into(left_idx + 1, mid, end, centroids);
        let node = &mut self.nodes[slot as usize];
        node.left_first = left_idx;
        node.count = 0;
    }

    fn range_bounds(&self, start: u32, end: u32) -> (Vec3, Vec3) {
        let mut bmin = Vec3::splat(f32::INFINITY);
        let mut bmax = Vec3::splat(f32::NEG_INFINITY);
        for &t in &self.tri_order[start as usize..end as usize] {
            for v in &self.tris[t as usize] {
                bmin = bmin.min(*v);
                bmax = bmax.max(*v);
            }
        }
        (bmin, bmax)
    }

    /// Squared distance from `p` to an AABB (0 inside).
    #[inline]
    fn aabb_dist_sq(p: Vec3, bmin: Vec3, bmax: Vec3) -> f32 {
        let dx = (bmin.x - p.x).max(p.x - bmax.x).max(0.0);
        let dy = (bmin.y - p.y).max(p.y - bmax.y).max(0.0);
        let dz = (bmin.z - p.z).max(p.z - bmax.z).max(0.0);
        dx * dx + dy * dy + dz * dz
    }

    /// Exact distance from `p` to the closest point on any triangle,
    /// via best-first branch-and-bound traversal.
    fn closest_distance(&self, p: Vec3) -> f32 {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;

        if self.nodes.is_empty() {
            return f32::INFINITY;
        }
        // Positive f32 bit patterns order identically to the values, so the
        // bit pattern doubles as a comparable heap key.
        let mut best = f32::INFINITY;
        let mut heap: BinaryHeap<(Reverse<u32>, u32)> = BinaryHeap::new();
        let root_bound = Self::aabb_dist_sq(p, self.nodes[0].bmin, self.nodes[0].bmax);
        heap.push((Reverse(root_bound.to_bits()), 0));

        while let Some((Reverse(bound_bits), ni)) = heap.pop() {
            // Every remaining node is at least this far away — done.
            if f32::from_bits(bound_bits) >= best {
                break;
            }
            let node = &self.nodes[ni as usize];
            if node.count > 0 {
                for &t in &self.tri_order
                    [node.left_first as usize..(node.left_first + node.count) as usize]
                {
                    let tri = &self.tris[t as usize];
                    let cp = closest_point_on_triangle(p, tri[0], tri[1], tri[2]);
                    let d2 = (cp - p).length_squared();
                    if d2 < best {
                        best = d2;
                    }
                }
            } else {
                for child in [node.left_first, node.left_first + 1] {
                    let c = &self.nodes[child as usize];
                    let b = Self::aabb_dist_sq(p, c.bmin, c.bmax);
                    if b < best {
                        heap.push((Reverse(b.to_bits()), child));
                    }
                }
            }
        }
        best.sqrt()
    }

    /// Counts ray-triangle intersections along the fixed parity direction.
    fn count_ray_hits(&self, origin: Vec3) -> u32 {
        let d = PARITY_DIR;
        let inv = Vec3::new(1.0 / d.x, 1.0 / d.y, 1.0 / d.z);
        let mut hits = 0u32;
        let mut stack = vec![0u32];
        while let Some(ni) = stack.pop() {
            let node = &self.nodes[ni as usize];
            if !ray_intersects_aabb(origin, inv, node.bmin, node.bmax) {
                continue;
            }
            if node.count > 0 {
                for &t in &self.tri_order
                    [node.left_first as usize..(node.left_first + node.count) as usize]
                {
                    let tri = &self.tris[t as usize];
                    if ray_triangle_t(origin, d, tri[0], tri[1], tri[2]).is_some() {
                        hits += 1;
                    }
                }
            } else {
                stack.push(node.left_first);
                stack.push(node.left_first + 1);
            }
        }
        hits
    }

    /// True when `p` is inside the closed mesh surface (odd ray parity).
    fn parity_inside(&self, p: Vec3) -> bool {
        self.count_ray_hits(p) % 2 == 1
    }
}

/// Ericson, *Real-Time Collision Detection* §5.1.6: closest point on a
/// triangle to `p`.
fn closest_point_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return a + ab * v;
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return a + ac * w;
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return b + (c - b) * w;
    }
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    a + ab * v + ac * w
}

/// Slab test between the ray `(origin, inv_dir)` and an AABB. `inv_dir` may
/// contain infinities for zero direction components (never NaN here because
/// the parity direction is component-wise non-zero).
fn ray_intersects_aabb(origin: Vec3, inv_dir: Vec3, bmin: Vec3, bmax: Vec3) -> bool {
    let mut tmin = 0.0f32;
    let mut tmax = f32::INFINITY;
    let o = [origin.x, origin.y, origin.z];
    let inv = [inv_dir.x, inv_dir.y, inv_dir.z];
    let mn = [bmin.x, bmin.y, bmin.z];
    let mx = [bmax.x, bmax.y, bmax.z];
    for a in 0..3 {
        let t1 = (mn[a] - o[a]) * inv[a];
        let t2 = (mx[a] - o[a]) * inv[a];
        tmin = tmin.max(t1.min(t2));
        tmax = tmax.min(t1.max(t2));
    }
    tmax >= tmin
}

/// Möller–Trumbore ray-triangle intersection; returns the ray parameter
/// `t > 1e-5` on hit.
fn ray_triangle_t(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let e1 = b - a;
    let e2 = c - a;
    let h = d.cross(e2);
    let det = e1.dot(h);
    if det.abs() < 1.0e-12 {
        return None;
    }
    let f = 1.0 / det;
    let s = o - a;
    let u = f * s.dot(h);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = f * d.dot(q);
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = f * e2.dot(q);
    (t > 1.0e-5).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::primitive::create_cube;

    #[test]
    fn baked_cube_signs_and_distances() {
        let cube = create_cube();
        let field = bake_mesh(&cube, Vec3::splat(2.0));
        assert_eq!(field.data.len(), 64 * 64 * 64);
        assert_eq!(field.world_size, Vec3::splat(2.0));

        // Texel centres sit at ±0.015625 m around the origin for this
        // domain, so none lands exactly on the surface (±0.5 m).
        // Domain-centre texel: 0.484375 m from every face → negative.
        let c = field.data[DistanceField64::index(32, 32, 32)];
        assert!(
            (c + 0.484_375).abs() < 1.0e-3,
            "interior centre texel should be ~-0.484375, got {c}"
        );

        // Half a texel beyond the +X face: exactly one texel edge out.
        let e = field.data[DistanceField64::index(63, 32, 32)];
        assert!(
            (e - 0.484_375).abs() < 1.0e-3,
            "exterior face texel should be ~+0.484375, got {e}"
        );

        // Outside along the diagonal: sqrt(3) times the axis distance.
        let k = field.data[DistanceField64::index(63, 63, 63)];
        let expected = 0.484_375f32 * 3.0f32.sqrt();
        assert!(
            (k - expected).abs() < 2.0e-3,
            "corner exterior texel should be ~{expected}, got {k}"
        );

        // Ray parity must classify exactly the 32³ interior texels as
        // negative — a strict whole-field invariant for the unit cube.
        let negatives = field.data.iter().filter(|&&d| d < 0.0).count();
        assert_eq!(negatives, 32 * 32 * 32, "wrong interior texel count");

        // The nearest texel centres sit one texel edge (0.015625 m) off
        // the faces — no zero crossings, no missed surface.
        let min_abs = field.data.iter().fold(f32::MAX, |a, &d| a.min(d.abs()));
        assert!(
            min_abs > 0.01 && min_abs < 0.03,
            "minimum |distance| should be about one texel, got {min_abs}"
        );
    }
}
