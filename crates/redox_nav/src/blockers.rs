//! Temporary blockers: volumes that make the polygons under them unwalkable until removed.
//!
//! The first version does not carve the baked tiles. A polygon is left out of the mesh when its
//! centre lies inside the blocker's footprint and within its height range, so the mesh is
//! rebuilt from the same tiles without re-baking. The result is coarse: a blocker is as sharp
//! as the polygons it covers.

use redox_math::Vec3;

use crate::data::{NavPolygon, NavTile};
use crate::error::NavError;
use crate::world::NavWorld;

/// Handle of a blocker added with [`NavWorld::add_blocker`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NavBlockerId(pub(crate) u32);

/// A footprint on the XZ plane with a height range.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Blocker {
    /// Footprint corners as (x, z), in order.
    pub(crate) footprint: Vec<[f32; 2]>,
    pub(crate) min_y: f32,
    pub(crate) max_y: f32,
}

impl NavWorld {
    /// Marks the footprint (a polygon on the XZ plane) between `min_y` and `max_y` unwalkable.
    ///
    /// # Errors
    ///
    /// [`NavError::BadBlocker`] for a footprint with fewer than 3 corners, non-finite values,
    /// or `min_y > max_y`.
    pub fn add_blocker(
        &mut self,
        footprint: &[Vec3],
        min_y: f32,
        max_y: f32,
    ) -> Result<NavBlockerId, NavError> {
        let finite = min_y.is_finite() && max_y.is_finite() && min_y <= max_y;
        if footprint.len() < 3 || !finite || footprint.iter().any(|p| !p.is_finite()) {
            return Err(NavError::BadBlocker);
        }
        let id = NavBlockerId(self.next_blocker);
        self.next_blocker = self
            .next_blocker
            .checked_add(1)
            .ok_or_else(|| NavError::Backend("blocker ids are exhausted".into()))?;
        let blocker = Blocker {
            footprint: footprint.iter().map(|p| [p.x, p.z]).collect(),
            min_y,
            max_y,
        };
        self.blockers.insert(id, blocker);
        self.rebuild_all();
        self.refresh();
        Ok(id)
    }

    /// Removes a blocker; returns whether it existed.
    pub fn remove_blocker(&mut self, id: NavBlockerId) -> bool {
        if self.blockers.remove(&id).is_none() {
            return false;
        }
        self.rebuild_all();
        self.refresh();
        true
    }

    /// Whether a blocker covers the centre of `polygon` (its height range included).
    pub(crate) fn blocker_covers(&self, tile: &NavTile, polygon: &NavPolygon) -> bool {
        if self.blockers.is_empty() {
            return false;
        }
        let corners: Vec<Vec3> = polygon
            .indices
            .iter()
            .filter_map(|&i| tile.vertices.get(usize::from(i)))
            .map(|v| Vec3::from_array(*v))
            .collect();
        if corners.is_empty() {
            return false;
        }
        let centre = corners.iter().copied().sum::<Vec3>() / corners.len() as f32;
        self.blockers.values().any(|blocker| {
            centre.y >= blocker.min_y
                && centre.y <= blocker.max_y
                && inside_footprint([centre.x, centre.z], &blocker.footprint)
        })
    }
}

/// Crossing-number test of a point against a polygon on the XZ plane.
fn inside_footprint(point: [f32; 2], footprint: &[[f32; 2]]) -> bool {
    let mut inside = false;
    let mut previous = footprint.len() - 1;
    for current in 0..footprint.len() {
        let [xi, zi] = footprint[current];
        let [xj, zj] = footprint[previous];
        let crosses = (zi > point[1]) != (zj > point[1]);
        if crosses && point[0] < (xj - xi) * (point[1] - zi) / (zj - zi) + xi {
            inside = !inside;
        }
        previous = current;
    }
    inside
}
