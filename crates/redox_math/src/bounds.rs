use crate::vector::Vec3;
use crate::matrix::Mat4;

/// Axis-Aligned Bounding Box
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    /// Creates an AABB from a center point and size (half-extents).
    ///
    /// # Arguments
    /// * `center` - The center of the bounding box
    /// * `half_extents` - The half-size of the box in each dimension
    ///
    /// # Returns
    /// A new AABB
    #[inline]
    pub fn from_center_size(center: Vec3, half_extents: Vec3) -> Self {
        Self {
            min: center - half_extents,
            max: center + half_extents,
        }
    }

    /// Creates an empty AABB (inside-out).
    #[inline]
    pub fn empty() -> Self {
        Self {
            min: Vec3::splat(f32::INFINITY),
            max: Vec3::splat(f32::NEG_INFINITY),
        }
    }

    /// Returns the center of the AABB.
    #[inline]
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// Returns the half-extents of the AABB.
    #[inline]
    pub fn half_extents(&self) -> Vec3 {
        (self.max - self.min) * 0.5
    }

    /// Returns the total size (full extents) of the AABB.
    #[inline]
    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }

    /// Transforms the AABB by a matrix and returns a new axis-aligned bounding box.
    ///
    /// This transforms all 8 corners of the box and computes a new AABB
    /// that contains all transformed corners.
    ///
    /// # Arguments
    /// * `matrix` - The transformation matrix
    ///
    /// # Returns
    /// A new AABB containing all transformed corners
    #[inline]
    pub fn transform(&self, matrix: Mat4) -> Self {
        let center = self.center();
        let half_extents = self.half_extents();

        // Transform all 8 corners
        let corners = [
            center + Vec3::new(-half_extents.x, -half_extents.y, -half_extents.z),
            center + Vec3::new(-half_extents.x, -half_extents.y, half_extents.z),
            center + Vec3::new(-half_extents.x, half_extents.y, -half_extents.z),
            center + Vec3::new(-half_extents.x, half_extents.y, half_extents.z),
            center + Vec3::new(half_extents.x, -half_extents.y, -half_extents.z),
            center + Vec3::new(half_extents.x, -half_extents.y, half_extents.z),
            center + Vec3::new(half_extents.x, half_extents.y, -half_extents.z),
            center + Vec3::new(half_extents.x, half_extents.y, half_extents.z),
        ];

        let mut result = Self::empty();
        for corner in corners {
            let transformed = matrix.transform_point3(corner);
            result = result.expand(transformed);
        }

        result
    }

    /// Expands the AABB to include a point.
    #[inline]
    pub fn expand(&self, point: Vec3) -> Self {
        Self {
            min: self.min.min(point),
            max: self.max.max(point),
        }
    }

    /// Checks if this AABB intersects another AABB.
    #[inline]
    pub fn intersects(&self, other: &Aabb) -> bool {
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
            && self.min.z <= other.max.z
            && self.max.z >= other.min.z
    }

    /// Checks if a point is inside this AABB.
    #[inline]
    pub fn contains_point(&self, point: Vec3) -> bool {
        point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
            && point.z >= self.min.z
            && point.z <= self.max.z
    }

    /// Gets the 8 corners of the AABB.
    #[inline]
    pub fn corners(&self) -> [Vec3; 8] {
        let min = self.min;
        let max = self.max;
        [
            Vec3::new(min.x, min.y, min.z),
            Vec3::new(min.x, min.y, max.z),
            Vec3::new(min.x, max.y, min.z),
            Vec3::new(min.x, max.y, max.z),
            Vec3::new(max.x, min.y, min.z),
            Vec3::new(max.x, min.y, max.z),
            Vec3::new(max.x, max.y, min.z),
            Vec3::new(max.x, max.y, max.z),
        ]
    }
}

impl Default for Aabb {
    fn default() -> Self {
        Self::empty()
    }
}

/// Bounding Sphere
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sphere {
    pub center: Vec3,
    pub radius: f32,
}

impl Sphere {
    /// Creates a new sphere from center and radius.
    #[inline]
    pub fn new(center: Vec3, radius: f32) -> Self {
        Self { center, radius }
    }

    /// Creates a sphere that contains an AABB.
    #[inline]
    pub fn from_aabb(aabb: &Aabb) -> Self {
        Self {
            center: aabb.center(),
            radius: aabb.half_extents().length(),
        }
    }

    /// Checks if a point is inside this sphere.
    #[inline]
    pub fn contains_point(&self, point: Vec3) -> bool {
        self.center.distance_squared(point) <= self.radius * self.radius
    }

    /// Checks if this sphere intersects another sphere.
    #[inline]
    pub fn intersects(&self, other: &Sphere) -> bool {
        let distance_squared = self.center.distance_squared(other.center);
        let radius_sum = self.radius + other.radius;
        distance_squared <= radius_sum * radius_sum
    }
}

impl Default for Sphere {
    fn default() -> Self {
        Self {
            center: Vec3::ZERO,
            radius: 0.0,
        }
    }
}
