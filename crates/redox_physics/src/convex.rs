//! Convex hulls for shape casts and overlap tests.

use rapier3d::prelude::SharedShape;
use redox_math::Vec3;

/// Fewest points that can span a volume.
const MIN_POINTS: usize = 4;

/// Smallest extent, in metres, every axis of a hull's bounding box must exceed.
const MIN_EXTENT: f32 = 1e-4;

/// A convex shape built from points, for sweeps and overlap tests. Cheap to clone (shared).
#[derive(Clone, Debug)]
pub struct ConvexHull {
    shape: SharedShape,
}

impl ConvexHull {
    /// The convex hull of `points`.
    ///
    /// Returns `None` when there are fewer than 4 points, any coordinate is not finite, or the
    /// points are (nearly) coplanar so the hull has no volume.
    pub fn from_points(points: &[Vec3]) -> Option<ConvexHull> {
        if points.len() < MIN_POINTS || !points.iter().all(|p| p.is_finite()) {
            return None;
        }
        if !spans_volume(points) {
            return None;
        }
        let shape = SharedShape::convex_hull(points)?;
        Some(Self { shape })
    }

    /// The rapier shape used by the physics queries.
    pub(crate) fn shared(&self) -> &SharedShape {
        &self.shape
    }
}

/// True when the bounding box of `points` exceeds [`MIN_EXTENT`] on every axis.
fn spans_volume(points: &[Vec3]) -> bool {
    let (min, max) = points.iter().fold(
        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(*p), hi.max(*p)),
    );
    let extent = max - min;
    extent.x > MIN_EXTENT && extent.y > MIN_EXTENT && extent.z > MIN_EXTENT
}
