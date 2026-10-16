use crate::vector::Vec3;
use crate::matrix::Mat4;

/// An axis-aligned bounding box defined by its minimum and maximum corners.
///
/// The box is considered to be closed (includes points exactly on its faces).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    /// The minimum corner (x, y, z).
    pub min: Vec3,
    /// The maximum corner (x, y, z).
    pub max: Vec3,
}

impl Aabb {
    /// Creates an AABB from a center point and half‑extents.
    ///
    /// # Arguments
    /// * `center` - The geometric center of the box.
    /// * `half_extents` - The distances from the center to each face.
    ///
    /// # Example
    /// ```
    /// # use redox_math::{Vec3, Aabb};
    /// let aabb = Aabb::from_center_size(Vec3::ZERO, Vec3::splat(2.0));
    /// assert_eq!(aabb.size(), Vec3::splat(4.0));
    /// ```
    pub fn from_center_size(center: Vec3, half_extents: Vec3) -> Self {
        Self {
            min: center - half_extents,
            max: center + half_extents,
        }
    }

    /// Creates an empty (invalid) AABB.
    ///
    /// The minimum corner is set to positive infinity and the maximum to
    /// negative infinity, so that any call to [`expand`](Self::expand) will
    /// produce a valid box containing at least that point.
    pub fn empty() -> Self {
        Self {
            min: Vec3::splat(f32::INFINITY),
            max: Vec3::splat(f32::NEG_INFINITY),
        }
    }

    /// Returns the center of the AABB.
    ///
    /// # Example
    /// ```
    /// # use redox_math::{Vec3, Aabb};
    /// let aabb = Aabb::from_center_size(Vec3::new(1.0, 2.0, 3.0), Vec3::splat(1.0));
    /// assert_eq!(aabb.center(), Vec3::new(1.0, 2.0, 3.0));
    /// ```
    #[inline]
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// Returns the half‑extents of the AABB (distance from center to each face).
    #[inline]
    pub fn half_extents(&self) -> Vec3 {
        (self.max - self.min) * 0.5
    }

    /// Returns the total size (width, height, depth) of the AABB.
    #[inline]
    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }

    /// Transforms this AABB by a 4×4 matrix and returns a new axis‑aligned
    /// bounding box that tightly encloses the transformed box.
    ///
    /// Uses Arvo's method (center transformed + per-axis abs-dot extents):
    /// 3 matrix-vector products instead of the 8 corner transforms.
    ///
    /// The result is always axis‑aligned in the target space.
    pub fn transform(&self, matrix: Mat4) -> Self {
        let center = self.center();
        let he = self.half_extents();

        let new_center = matrix.transform_point3(center);

        // |M_ij| * extent_j summed over j, ignoring translation column.
        let m = matrix.to_cols_array_2d();
        let new_he = Vec3::new(
            m[0][0].abs() * he.x + m[1][0].abs() * he.y + m[2][0].abs() * he.z,
            m[0][1].abs() * he.x + m[1][1].abs() * he.y + m[2][1].abs() * he.z,
            m[0][2].abs() * he.x + m[1][2].abs() * he.y + m[2][2].abs() * he.z,
        );

        Self::from_center_size(new_center, new_he)
    }

    /// Expands the AABB to include the given point.
    ///
    /// Returns a new AABB that is the union of the current box and the point.
    pub fn expand(&self, point: Vec3) -> Self {
        Self {
            min: self.min.min(point),
            max: self.max.max(point),
        }
    }

    /// Checks whether the AABB contains a given point.
    ///
    /// Points exactly on the faces are considered inside.
    pub fn contains_point(&self, point: Vec3) -> bool {
        point.x >= self.min.x && point.x <= self.max.x &&
            point.y >= self.min.y && point.y <= self.max.y &&
            point.z >= self.min.z && point.z <= self.max.z
    }
}

/// A sphere defined by a center and a radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sphere {
    /// The center of the sphere.
    pub center: Vec3,
    /// The radius of the sphere.
    pub radius: f32,
}

impl Sphere {
    /// Creates a new sphere from a center and a radius.
    ///
    /// # Arguments
    /// * `center` - The center point.
    /// * `radius` - The radius (must be non‑negative for meaningful geometry).
    pub fn new(center: Vec3, radius: f32) -> Self {
        Self { center, radius }
    }

    /// Tests if this sphere intersects another sphere.
    ///
    /// Two spheres intersect if the distance between their centers is less than
    /// or equal to the sum of their radii.
    pub fn intersects(&self, other: &Sphere) -> bool {
        let distance_squared = self.center.distance_squared(other.center);
        let radius_sum = self.radius + other.radius;
        distance_squared <= radius_sum * radius_sum
    }
}