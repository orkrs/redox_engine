//! Shadow-atlas tile allocation for point-light cube shadows.
//!
//! The atlas is a single square depth texture divided into a uniform grid of
//! `TILE_PX × TILE_PX` tiles. Each shadow-casting point light consumes **6**
//! consecutive tiles (one cube face each). Allocation priority is nearest-to-
//! camera first; lights that do not fit keep `NO_TILE` and render unshadowed.

/// Edge length of one tile in pixels.
pub const TILE_PX: u32 = 512;
/// Atlas edge length in pixels (must be a multiple of [`TILE_PX`]).
pub const ATLAS_PX: u32 = 8192;
/// Tiles per row/column.
pub const TILES_PER_AXIS: u32 = ATLAS_PX / TILE_PX;
/// Total tile count.
pub const TOTAL_TILES: u32 = TILES_PER_AXIS * TILES_PER_AXIS;
/// Sentinel written into `PointLightGpu::shadow_tile_base` for unshadowed lights.
pub const NO_TILE: u32 = u32::MAX;

/// Maximum simultaneous shadow casters (6 tiles each).
pub const MAX_CASTERS: u32 = TOTAL_TILES / 6;

/// Allocates consecutive 6-tile runs for point-light casters.
#[derive(Debug, Default)]
pub struct ShadowAtlasAllocator {
    next_free: u32,
    /// `(entity_hint, allocated_base)` pairs from the current frame.
    live_allocations: Vec<(u64, u32)>,
}

impl ShadowAtlasAllocator {
    /// Creates an allocator with every tile free.
    pub fn new() -> Self {
        Self {
            next_free: 0,
            live_allocations: Vec::new(),
        }
    }

    /// Starts a fresh frame: resets the bump pointer.
    ///
    /// Stability heuristic: callers sort casters by distance to camera BEFORE
    /// allocating, so the closest lights win when the atlas saturates. Because
    /// ordering changes smoothly across frames, tiles stay roughly stable.
    pub fn begin_frame(&mut self) {
        self.next_free = 0;
        self.live_allocations.clear();
    }

    /// Allocates a 6-tile run. Returns the base tile index or [`NO_TILE`].
    pub fn allocate(&mut self, entity_hint: u64) -> u32 {
        if self.next_free + 6 > TOTAL_TILES {
            return NO_TILE;
        }
        let base = self.next_free;
        self.next_free += 6;
        self.live_allocations.push((entity_hint, base));
        base
    }

    /// Number of casters currently allocated.
    pub fn allocated_casters(&self) -> u32 {
        self.next_free / 6
    }
}

/// Tile-grid column/row for a tile index.
pub fn tile_grid(tile_index: u32) -> (u32, u32) {
    (tile_index % TILES_PER_AXIS, tile_index / TILES_PER_AXIS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocates_runs_and_saturates() {
        let mut alloc = ShadowAtlasAllocator::new();
        assert_eq!(alloc.allocate(1), 0);
        assert_eq!(alloc.allocate(2), 6);
        assert_eq!(MAX_CASTERS, 42);
        // Fill to saturation.
        for i in 0..40 {
            let _ = alloc.allocate(10 + i);
        }
        assert_eq!(alloc.allocate(999), NO_TILE);
        alloc.begin_frame();
        assert_eq!(alloc.allocate(5), 0);
        assert_eq!(alloc.allocated_casters(), 1);
    }
}
