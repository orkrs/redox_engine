//! Toroidal probe cascade: a fixed-size probe lattice that follows the camera in whole cells.
//!
//! World cell `c` of a probe lies at `origin + c * spacing`, with `c = floor((p - origin) / spacing)`
//! for the nearest lattice point. The cascade window covers the cells `[scroll, scroll + dims)`.
//! Probes are stored in a ring: storage slot `c.rem_euclid(dims)` per axis, so moving the
//! window never moves the data, only the mapping from slots to world cells.

use redox_math::Vec3;

/// One probe cascade: origin, spacing, size and the current window offset in cells.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProbeCascade {
    /// Reference world position; cell `(0, 0, 0)` starts here.
    pub origin: Vec3,
    /// Distance between neighbouring probes in metres.
    pub spacing: f32,
    /// Probe counts per axis.
    pub dims: [u32; 3],
    /// Lowest world cell of the window, per axis.
    pub scroll: [i32; 3],
}

impl ProbeCascade {
    /// Cascade with its window at cell zero (`scroll = 0`).
    pub fn new(origin: Vec3, spacing: f32, dims: [u32; 3]) -> Self {
        Self {
            origin,
            spacing,
            dims,
            scroll: [0; 3],
        }
    }

    /// Centres the window on the camera in whole cells.
    ///
    /// Returns the shift applied by this call, in cells per axis (the change of `scroll`).
    /// ASSUMPTION: the spec does not define the window anchor; here the window is centred
    /// on the camera's cell, with `origin` as the fixed reference.
    pub fn snap_to_camera(&mut self, cam_pos: Vec3) -> [i32; 3] {
        let local = ((cam_pos - self.origin) / self.spacing).floor().to_array();
        let mut shift = [0; 3];
        for axis in 0..3 {
            let half = (self.dims[axis] / 2) as i32;
            let target = local[axis] as i32 - half;
            shift[axis] = target - self.scroll[axis];
            self.scroll[axis] = target;
        }
        shift
    }

    /// Storage slot of a world cell (ring-wrapped per axis).
    pub fn storage_index(&self, cell: [i32; 3]) -> u32 {
        let [dx, dy, _] = self.dims;
        let x = cell[0].rem_euclid(self.dims[0] as i32) as u32;
        let y = cell[1].rem_euclid(self.dims[1] as i32) as u32;
        let z = cell[2].rem_euclid(self.dims[2] as i32) as u32;
        x + y * dx + z * dx * dy
    }

    /// World position of the probe stored in `index`, from the window it currently covers.
    pub fn probe_world_pos(&self, index: u32) -> Vec3 {
        let [dx, dy, _] = self.dims;
        let slots = [index % dx, (index / dx) % dy, index / (dx * dy)];
        let cell = [
            self.cell_of_slot(0, slots[0]),
            self.cell_of_slot(1, slots[1]),
            self.cell_of_slot(2, slots[2]),
        ];
        self.origin + Vec3::new(cell[0] as f32, cell[1] as f32, cell[2] as f32) * self.spacing
    }

    /// Number of probes in the cascade.
    pub fn probe_count(&self) -> u32 {
        self.dims[0] * self.dims[1] * self.dims[2]
    }

    /// World cell in the current window that maps to ring slot `slot` on `axis`.
    fn cell_of_slot(&self, axis: usize, slot: u32) -> i32 {
        let dim = self.dims[axis] as i32;
        self.scroll[axis] + (slot as i32 - self.scroll[axis]).rem_euclid(dim)
    }
}
