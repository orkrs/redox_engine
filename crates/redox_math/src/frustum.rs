//! Frustum culling primitives.
//!
//! This module provides a plane representation and a frustum structure
//! that can be extracted from a view‑projection matrix. The frustum can
//! then be used for fast intersection tests with bounding volumes such
//! as AABBs and spheres.

use crate::bounds::Aabb;
use crate::matrix::Mat4;
use crate::vector::Vec3;

/// A plane in 3D space defined by the equation `normal · point + distance = 0`.
///
/// The plane splits space into a positive half‑space (where the signed distance
/// is positive) and a negative half‑space. The normal points toward the positive side.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Plane {
    /// Unit‑length normal vector of the plane (after normalization).
    pub normal: Vec3,
    /// Signed distance from the origin along the normal to the plane.
    /// A point `p` lies on the plane if `normal · p + distance = 0`.
    pub distance: f32,
}

impl Plane {
    /// Creates a new plane from a normal and a distance.
    ///
    /// The normal does **not** need to be normalized initially; it can be
    /// normalized later with [`normalize`](Self::normalize).
    ///
    /// # Arguments
    /// * `normal`   – The normal vector (may be non‑unit).
    /// * `distance` – The signed distance from the origin along the normal.
    ///
    /// # Example
    /// ```
    /// # use redox_math::{Vec3, Plane};
    /// let plane = Plane::new(Vec3::new(1.0, 2.0, 3.0), 5.0);
    /// ```
    #[inline]
    pub fn new(normal: Vec3, distance: f32) -> Self {
        Self { normal, distance }
    }

    /// Normalises the plane so that its normal becomes a unit vector.
    ///
    /// This also scales the distance accordingly, preserving the plane equation.
    /// If the normal length is zero (degenerate plane), this method does nothing.
    #[inline]
    pub fn normalize(&mut self) {
        let length = self.normal.length();
        if length > 0.0 {
            let inv_length = 1.0 / length;
            self.normal *= inv_length;
            self.distance *= inv_length;
        }
    }

    /// Returns the signed distance from a point to the plane.
    ///
    /// Positive values indicate the point is in front of the plane
    /// (in the direction of the normal), negative values indicate it is behind.
    #[inline]
    pub fn dot_point(&self, point: Vec3) -> f32 {
        self.normal.dot(point) + self.distance
    }
}

/// A viewing frustum defined by six planes.
///
/// The planes are stored in the following order:
/// 0: Left, 1: Right, 2: Bottom, 3: Top, 4: Near, 5: Far.
/// All planes are oriented so that their normals point outward (away from the
/// interior of the frustum). An object is inside the frustum if it lies on the
/// positive side (i.e., signed distance ≥ 0) of every plane.
pub struct Frustum {
    /// The six planes of the frustum.
    pub planes: [Plane; 6],
}

impl Frustum {
    /// Extracts frustum planes from a view‑projection matrix using the
    /// Gribb‑Hartmann method.
    ///
    /// Assumes **wgpu / DirectX-style clip space** with depth range `[0, 1]`
    /// (projections created by [`perspective`](crate::matrix::perspective)).
    /// The near plane is taken directly from row 3 (`m.row(2)`); an
    /// OpenGL-style `[-1, 1]` projection would require `row3 + row2` instead
    /// and is NOT supported by this constructor.
    ///
    /// # Arguments
    /// * `m` – The combined view‑projection matrix (projection * view).
    ///
    /// # Returns
    /// A `Frustum` with all six planes normalised.
    ///
    /// # Example
    /// ```
    /// # use redox_math::{Mat4, perspective, look_at, Vec3, Frustum};
    /// let proj = perspective(90.0_f32.to_radians(), 16.0/9.0, 0.1, 100.0);
    /// let view = look_at(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
    /// let frustum = Frustum::from_view_projection(proj * view);
    /// ```
    pub fn from_view_projection(m: Mat4) -> Self {
        let row1 = m.row(0);
        let row2 = m.row(1);
        let row3 = m.row(2);
        let row4 = m.row(3);

        // Helper to create a plane from a Vec4 (row combination)
        // Extracting X, Y, Z for the normal and W for distance
        let make_plane = |v: glam::Vec4| Plane::new(Vec3::new(v.x, v.y, v.z), v.w);

        let mut frustum = Self {
            planes: [
                make_plane(row4 + row1), // Left
                make_plane(row4 - row1), // Right
                make_plane(row4 + row2), // Bottom
                make_plane(row4 - row2), // Top
                make_plane(row3),        // Near (Directly from row3 for [0, 1] depth range)
                make_plane(row4 - row3), // Far
            ],
        };

        for plane in &mut frustum.planes {
            plane.normalize();
        }
        frustum
    }

    /// Tests whether an axis‑aligned bounding box (AABB) intersects the frustum.
    ///
    /// This is a conservative test: it returns `true` if the AABB is at least
    /// partially inside the frustum, and `false` if it is definitely outside.
    ///
    /// The algorithm uses the “p‑vertex” method: for each plane, the corner of
    /// the AABB that is most in the direction of the plane’s normal is tested.
    /// If that corner is behind the plane, the entire AABB is outside.
    ///
    /// # Arguments
    /// * `aabb` – The AABB to test.
    ///
    /// # Returns
    /// `true` if the AABB intersects or is fully inside the frustum,
    /// `false` if it is completely outside.
    pub fn intersects_aabb(&self, aabb: &Aabb) -> bool {
        for plane in &self.planes {
            let min = aabb.min;
            let max = aabb.max;

            // P-vertex: find the corner most in the direction of the normal
            let mut p = min;
            if plane.normal.x >= 0.0 {
                p.x = max.x;
            }
            if plane.normal.y >= 0.0 {
                p.y = max.y;
            }
            if plane.normal.z >= 0.0 {
                p.z = max.z;
            }

            // If the p-vertex is on the negative side of the plane, the AABB is outside
            if plane.dot_point(p) < 0.0 {
                return false;
            }
        }
        true
    }

    /// Tests whether a sphere intersects the frustum.
    ///
    /// Conservative p-vertex-style test: for each plane, the signed distance of
    /// the sphere center is compared against the radius. Returns `false` only
    /// when the sphere is definitely outside at least one plane.
    pub fn intersects_sphere(&self, sphere: &crate::bounds::Sphere) -> bool {
        for plane in &self.planes {
            if plane.dot_point(sphere.center) < -sphere.radius {
                return false;
            }
        }
        true
    }
}
