use crate::vector::{Vec3, Vec4};
use crate::matrix::Mat4;
use crate::bounds::Aabb;

/// A plane in 3D space represented by its normal and distance from the origin.
/// The plane equation is `dot(normal, point) + distance = 0`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Plane {
    pub normal: Vec3,
    pub distance: f32,
}

impl Plane {
    /// Creates a new plane.
    #[inline]
    pub fn new(normal: Vec3, distance: f32) -> Self {
        Self { normal, distance }
    }

    /// Normalizes the plane's normal and scales the distance accordingly.
    #[inline]
    pub fn normalize(&mut self) {
        let length = self.normal.length();
        if length > 0.0 {
            let inv_length = 1.0 / length;
            self.normal *= inv_length;
            self.distance *= inv_length;
        }
    }

    /// Returns the dot product of the plane and a point.
    /// Positive value means point is in front of the plane,
    /// negative means behind, zero means on the plane.
    #[inline]
    pub fn dot_point(&self, point: Vec3) -> f32 {
        self.normal.dot(point) + self.distance
    }
}

/// A viewing frustum defined by 6 planes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frustum {
    /// Planes of the frustum in order: Left, Right, Bottom, Top, Near, Far.
    pub planes: [Plane; 6],
}

impl Frustum {
    /// Extracts a frustum from a view-projection matrix.
    ///
    /// This assumes a right-handed coordinate system and clip space depth range [0, 1] (WGPU/D3D).
    /// If using OpenGL, the extraction for near/far planes might differ.
    pub fn from_view_projection(m: Mat4) -> Self {
        let row4 = m.row(3);
        let row1 = m.row(0);
        let row2 = m.row(1);
        let row3 = m.row(2);

        let mut frustum = Self {
            planes: [
                // Left:   col4 + col1
                Plane::new(Vec3::from(row4 + row1), row4.w + row1.w),
                // Right:  col4 - col1
                Plane::new(Vec3::from(row4 - row1), row4.w - row1.w),
                // Bottom: col4 + col2
                Plane::new(Vec3::from(row4 + row2), row4.w + row2.w),
                // Top:    col4 - col2
                Plane::new(Vec3::from(row4 - row2), row4.w - row2.w),
                // Near:   col3 (using [0, 1] range)
                Plane::new(Vec3::from(row3), row3.w),
                // Far:    col4 - col3
                Plane::new(Vec3::from(row4 - row3), row4.w - row3.w),
            ],
        };

        for plane in &mut frustum.planes {
            plane.normalize();
        }

        frustum
    }

    /// Checks if an AABB intersects or is inside the frustum.
    ///
    /// Returns true if the AABB is partially or fully inside.
    pub fn intersects_aabb(&self, aabb: &Aabb) -> bool {
        for plane in &self.planes {
            let min = aabb.min;
            let max = aabb.max;

            // Find the positive vertex (most in direction of the normal)
            let mut p = min;
            if plane.normal.x >= 0.0 { p.x = max.x; }
            if plane.normal.y >= 0.0 { p.y = max.y; }
            if plane.normal.z >= 0.0 { p.z = max.z; }

            // If the positive vertex is behind the plane, the whole AABB is outside
            if plane.dot_point(p) < 0.0 {
                return false;
            }
        }
        true
    }
}
