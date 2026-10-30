//! Streaming of the GI distance-field bricks by world cell (GI-06).
//!
//! The pure logic lives here: which cells are inside the streaming radius (with a hysteresis
//! so a camera on the border does not flap), where the sliding window of the global SDF sits,
//! the brick budget, and when a rebuild may be requested. Reading the `.sdf` files of the
//! chosen cells is [`load_cell_bricks`]; the game applies the result with
//! `Renderer::set_gi_static_bricks`.

use std::collections::BTreeSet;
use std::path::Path;

use redox_math::Vec3;
use redox_render::sdf::atlas::SdfBrick;
use redox_render::sdf::format::read_bricks;

/// A world cell as `(x, y)`; the cell covers `[x * size, (x + 1) * size)` along world X and
/// `[y * size, (y + 1) * size)` along world Z.
pub type Cell = (i32, i32);

/// Tuning of the streaming.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GiStreamingSettings {
    /// Cells whose centre is closer than this to the camera are loaded, metres.
    pub radius: f32,
    /// A loaded cell is kept until it is this much farther than `radius`.
    pub hysteresis: f32,
    /// At most this many bricks in the window; the farthest are dropped first.
    pub max_bricks: usize,
    /// Shortest time between two rebuild requests, seconds.
    pub min_interval: f32,
}

impl Default for GiStreamingSettings {
    fn default() -> Self {
        Self {
            radius: 96.0,
            hysteresis: 16.0,
            max_bricks: 4096,
            min_interval: 0.5,
        }
    }
}

fn cell_centre(cell: Cell, cell_size: f32) -> (f32, f32) {
    (
        (cell.0 as f32 + 0.5) * cell_size,
        (cell.1 as f32 + 0.5) * cell_size,
    )
}

fn distance(camera: Vec3, cell: Cell, cell_size: f32) -> f32 {
    let (cx, cz) = cell_centre(cell, cell_size);
    ((camera.x - cx).powi(2) + (camera.z - cz).powi(2)).sqrt()
}

/// The cells to keep loaded: every cell of `available` within `radius` of the camera, plus
/// the cells of `loaded` that are still within `radius + hysteresis`. Sorted by distance
/// (nearest first), ties by cell, so the result is deterministic.
pub fn select_cells(
    camera: Vec3,
    available: &[Cell],
    loaded: &BTreeSet<Cell>,
    cell_size: f32,
    settings: &GiStreamingSettings,
) -> Vec<Cell> {
    let mut chosen: Vec<(f32, Cell)> = available
        .iter()
        .filter_map(|&cell| {
            let d = distance(camera, cell, cell_size);
            let limit = if loaded.contains(&cell) {
                settings.radius + settings.hysteresis
            } else {
                settings.radius
            };
            (d <= limit).then_some((d, cell))
        })
        .collect();
    chosen.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    chosen.into_iter().map(|(_, cell)| cell).collect()
}

/// Origin of the sliding SDF window for a camera at `camera`: the window corner snapped to a
/// multiple of `voxel * 8`, so small camera moves do not shift the voxel grid.
pub fn window_origin(camera: Vec3, window_size: Vec3, voxel: f32) -> Vec3 {
    let step = voxel * 8.0;
    ((camera - window_size * 0.5) / step).floor() * step
}

/// Keeps at most `max_bricks` bricks, dropping the farthest from `camera` first.
/// Returns how many were dropped.
pub fn limit_bricks(bricks: &mut Vec<SdfBrick>, camera: Vec3, max_bricks: usize) -> usize {
    if bricks.len() <= max_bricks {
        return 0;
    }
    bricks.sort_by(|a, b| {
        (a.origin_ws - camera)
            .length_squared()
            .total_cmp(&(b.origin_ws - camera).length_squared())
    });
    let dropped = bricks.len() - max_bricks;
    bricks.truncate(max_bricks);
    dropped
}

/// Decides when the loaded set changes and a rebuild is due.
#[derive(Debug, Default)]
pub struct GiSdfStreamer {
    loaded: BTreeSet<Cell>,
    since_rebuild: f32,
}

impl GiSdfStreamer {
    /// A streamer with nothing loaded; the first call that selects cells asks for a rebuild.
    pub fn new() -> Self {
        Self {
            loaded: BTreeSet::new(),
            since_rebuild: f32::MAX,
        }
    }

    /// Cells currently loaded.
    pub fn loaded(&self) -> &BTreeSet<Cell> {
        &self.loaded
    }

    /// Advances by `dt` seconds. Returns the new cell set when it differs from the loaded one
    /// and at least `min_interval` has passed since the last rebuild; `None` otherwise.
    pub fn update(
        &mut self,
        dt: f32,
        camera: Vec3,
        available: &[Cell],
        cell_size: f32,
        settings: &GiStreamingSettings,
    ) -> Option<Vec<Cell>> {
        self.since_rebuild = (self.since_rebuild + dt).min(1.0e9);
        let wanted = select_cells(camera, available, &self.loaded, cell_size, settings);
        let wanted_set: BTreeSet<Cell> = wanted.iter().copied().collect();
        if wanted_set == self.loaded || self.since_rebuild < settings.min_interval {
            return None;
        }
        self.loaded = wanted_set;
        self.since_rebuild = 0.0;
        Some(wanted)
    }
}

/// Reads and concatenates the bricks of `cells` from `<dir>/<x>_<y>.sdf`. A cell without a
/// file contributes nothing; each cell is read once even if listed twice.
///
/// # Errors
/// A message naming the file when one exists but cannot be read or decoded.
pub fn load_cell_bricks(dir: &Path, cells: &[Cell]) -> Result<Vec<SdfBrick>, String> {
    let mut seen = BTreeSet::new();
    let mut bricks = Vec::new();
    for &cell in cells {
        if !seen.insert(cell) {
            continue;
        }
        let path = dir.join(format!("{}_{}.sdf", cell.0, cell.1));
        if !path.is_file() {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        bricks.extend(read_bricks(&bytes).map_err(|e| format!("{}: {e}", path.display()))?);
    }
    Ok(bricks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_render::sdf::bake::DistanceField64;

    const SIZE: f32 = 64.0;

    fn grid(n: i32) -> Vec<Cell> {
        (-n..=n)
            .flat_map(|x| (-n..=n).map(move |y| (x, y)))
            .collect()
    }

    #[test]
    fn cells_within_the_radius_are_selected_and_sorted_by_distance() {
        let settings = GiStreamingSettings::default();
        let chosen = select_cells(
            Vec3::new(32.0, 0.0, 32.0),
            &grid(5),
            &BTreeSet::new(),
            SIZE,
            &settings,
        );
        assert_eq!(chosen[0], (0, 0));
        assert!(chosen.len() > 1 && chosen.len() < grid(5).len());
        let dists: Vec<f32> = chosen
            .iter()
            .map(|&c| distance(Vec3::new(32.0, 0.0, 32.0), c, SIZE))
            .collect();
        assert!(dists.windows(2).all(|w| w[0] <= w[1]));
        assert!(dists.iter().all(|&d| d <= settings.radius));
    }

    #[test]
    fn hysteresis_prevents_flapping_at_the_boundary() {
        let settings = GiStreamingSettings::default();
        let cells = [(0, 0), (3, 0)];
        let mut streamer = GiSdfStreamer::new();
        let mut changes = 0;
        for i in 0..100 {
            // The camera wobbles by ±3 m around the radius of the second cell.
            let x = 32.0 + 3.5 * 64.0 - 96.0 + if i % 2 == 0 { -3.0 } else { 3.0 };
            if streamer
                .update(1.0, Vec3::new(x, 0.0, 32.0), &cells, SIZE, &settings)
                .is_some()
            {
                changes += 1;
            }
        }
        assert!(changes <= 2, "{changes} changes");
    }

    #[test]
    fn the_window_origin_snaps_to_the_voxel_grid() {
        let voxel = 0.5;
        let size = Vec3::splat(64.0);
        let a = window_origin(Vec3::new(100.1, 0.0, 0.0), size, voxel);
        let b = window_origin(Vec3::new(100.9, 0.0, 0.0), size, voxel);
        assert_eq!(a, b);
        assert_eq!(a.x % (voxel * 8.0), 0.0);
    }

    fn brick_at(x: f32) -> SdfBrick {
        SdfBrick {
            field: DistanceField64 {
                data: Vec::new(),
                world_size: Vec3::ONE,
            },
            origin_ws: Vec3::new(x, 0.0, 0.0),
            world_size: Vec3::ONE,
        }
    }

    #[test]
    fn the_brick_limit_drops_the_farthest_first() {
        let mut bricks: Vec<SdfBrick> = [5.0, 1.0, 9.0, 3.0].map(brick_at).into();
        let dropped = limit_bricks(&mut bricks, Vec3::ZERO, 2);
        assert_eq!(dropped, 2);
        let mut xs: Vec<f32> = bricks.iter().map(|b| b.origin_ws.x).collect();
        xs.sort_by(f32::total_cmp);
        assert_eq!(xs, vec![1.0, 3.0]);
    }

    #[test]
    fn a_rebuild_is_not_requested_when_the_set_is_unchanged() {
        let settings = GiStreamingSettings::default();
        let cells = [(0, 0)];
        let mut streamer = GiSdfStreamer::new();
        assert!(
            streamer
                .update(1.0, Vec3::new(32.0, 0.0, 32.0), &cells, SIZE, &settings)
                .is_some()
        );
        for _ in 0..10 {
            assert!(
                streamer
                    .update(1.0, Vec3::new(32.0, 0.0, 32.0), &cells, SIZE, &settings)
                    .is_none()
            );
        }
    }

    #[test]
    fn loading_the_same_cell_twice_does_not_duplicate_bricks() {
        let dir = std::env::temp_dir().join(format!("redox_gi_stream_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut brick = brick_at(2.0);
        brick.field.data = vec![0.5; 64 * 64 * 64];
        std::fs::write(
            dir.join("0_0.sdf"),
            redox_render::sdf::format::write_bricks(&[brick]),
        )
        .unwrap();
        let bricks = load_cell_bricks(&dir, &[(0, 0), (0, 0), (9, 9)]).unwrap();
        assert_eq!(bricks.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
