//! Uniform hash grid on the x-z plane: finds the agents near a point without visiting all of them.

use std::collections::HashMap;

/// One stored point: the caller's id and its position on the plane.
#[derive(Clone, Copy, Debug)]
struct Entry {
    index: usize,
    x: f32,
    z: f32,
}

/// Square cells of `cell_size` metres on the x-z plane, each holding the points inside it.
///
/// A query with radius `r` visits only the cells that the circle touches, so its cost follows the
/// number of points near the query, not the number of points in the grid.
pub struct UniformGrid {
    cell_size: f32,
    cells: HashMap<(i32, i32), Vec<Entry>>,
}

impl UniformGrid {
    /// An empty grid with cells of `cell_size` metres. Returns `None` unless `cell_size` is
    /// positive and finite.
    pub fn new(cell_size: f32) -> Option<Self> {
        (cell_size.is_finite() && cell_size > 0.0).then(|| Self {
            cell_size,
            cells: HashMap::new(),
        })
    }

    /// Removes every point. The cell size is kept.
    pub fn clear(&mut self) {
        self.cells.clear();
    }

    /// Stores the point `index` at (`x`, `z`).
    pub fn insert(&mut self, index: usize, x: f32, z: f32) {
        let key = self.cell_of(x, z);
        self.cells
            .entry(key)
            .or_default()
            .push(Entry { index, x, z });
    }

    /// Appends to `out` the ids of the points within `radius` of (`x`, `z`), boundary included.
    /// The order is unspecified. A negative or non-finite radius finds nothing.
    pub fn query(&self, x: f32, z: f32, radius: f32, out: &mut Vec<usize>) {
        if !(radius.is_finite() && radius >= 0.0) {
            return;
        }
        let (min_x, min_z) = self.cell_of(x - radius, z - radius);
        let (max_x, max_z) = self.cell_of(x + radius, z + radius);
        let radius_squared = radius * radius;
        for cell_x in min_x..=max_x {
            for cell_z in min_z..=max_z {
                let Some(entries) = self.cells.get(&(cell_x, cell_z)) else {
                    continue;
                };
                for entry in entries {
                    let (dx, dz) = (entry.x - x, entry.z - z);
                    if dx * dx + dz * dz <= radius_squared {
                        out.push(entry.index);
                    }
                }
            }
        }
    }

    /// Cell that holds the plane point (`x`, `z`).
    fn cell_of(&self, x: f32, z: f32) -> (i32, i32) {
        (
            (x / self.cell_size).floor() as i32,
            (z / self.cell_size).floor() as i32,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::UniformGrid;

    /// Deterministic pseudo-random points in [-20, 20) on both axes.
    fn points(count: usize) -> Vec<(f32, f32)> {
        let mut state = 7u32;
        let mut next = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 8) as f32 / 16_777_216.0
        };
        (0..count)
            .map(|_| ((next() - 0.5) * 40.0, (next() - 0.5) * 40.0))
            .collect()
    }

    #[test]
    fn the_uniform_grid_returns_exactly_the_agents_within_the_radius() {
        let pts = points(300);
        let mut grid = UniformGrid::new(2.0).expect("positive cell size");
        for (i, &(x, z)) in pts.iter().enumerate() {
            grid.insert(i, x, z);
        }
        for &(qx, qz) in &points(50) {
            for radius in [0.5_f32, 2.0, 5.0] {
                let mut found = Vec::new();
                grid.query(qx, qz, radius, &mut found);
                found.sort_unstable();
                let brute: Vec<usize> = (0..pts.len())
                    .filter(|&i| {
                        let (dx, dz) = (pts[i].0 - qx, pts[i].1 - qz);
                        dx * dx + dz * dz <= radius * radius
                    })
                    .collect();
                assert_eq!(found, brute, "query at ({qx}, {qz}), radius {radius}");
            }
        }
    }

    #[test]
    fn a_bad_cell_size_is_rejected() {
        assert!(UniformGrid::new(0.0).is_none());
        assert!(UniformGrid::new(-1.0).is_none());
        assert!(UniformGrid::new(f32::NAN).is_none());
    }

    #[test]
    fn clear_forgets_every_point() {
        let mut grid = UniformGrid::new(1.0).expect("positive cell size");
        grid.insert(0, 0.5, 0.5);
        grid.clear();
        let mut found = Vec::new();
        grid.query(0.5, 0.5, 1.0, &mut found);
        assert!(found.is_empty());
    }
}
