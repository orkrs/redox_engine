//! Page cache — tracks which physical pages are clean (up-to-date) and
//! manages invalidation when objects or lights move.

use super::page_table::{PAGE_CACHED_BIT, PAGE_PHYS_MASK, PAGE_VALID_BIT, VirtualPageTable};

/// Invalidation event: something moved that could affect shadow data.
#[derive(Clone, Debug)]
pub enum InvalidationEvent {
    /// The directional light direction changed.
    LightMoved,
    /// A dynamic object's bounding box intersects a set of virtual pages.
    DynamicObject {
        /// Affected clipmap level index.
        level: u32,
        /// Affected virtual page range (inclusive min/max).
        page_min_x: u32,
        page_min_y: u32,
        page_max_x: u32,
        page_max_y: u32,
    },
    /// Complete invalidation (e.g. first frame, config change).
    Full,
}

/// Processes invalidation events, marking affected pages as dirty (clearing
/// the `CACHED` bit) so they are re-rendered next frame.
pub fn process_invalidations(
    events: &[InvalidationEvent],
    page_tables: &mut [VirtualPageTable],
) {
    for event in events {
        match event {
            InvalidationEvent::Full | InvalidationEvent::LightMoved => {
                for pt in page_tables.iter_mut() {
                    pt.invalidate_all();
                }
            }
            InvalidationEvent::DynamicObject {
                level,
                page_min_x,
                page_min_y,
                page_max_x,
                page_max_y,
            } => {
                if let Some(pt) = page_tables.get_mut(*level as usize) {
                    for y in *page_min_y..=(*page_max_y).min(pt.pages_y - 1) {
                        for x in *page_min_x..=(*page_max_x).min(pt.pages_x - 1) {
                            let entry = pt.get(x, y);
                            if entry & PAGE_VALID_BIT != 0 {
                                pt.set(x, y, entry & !PAGE_CACHED_BIT);
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Collects dirty (valid but not cached) pages that need re-rendering.
pub struct DirtyPageCollector;

impl DirtyPageCollector {
    /// Returns `(level, page_x, page_y, phys_idx)` for every page that has
    /// been requested by the visibility pass but whose CACHED bit is clear.
    pub fn collect(
        page_tables: &[VirtualPageTable],
        requested_bits: &[u32],
    ) -> Vec<(u32, u32, u32, u16)> {
        let mut dirty = Vec::new();
        let mut bit_offset = 0u32;

        for (level, pt) in page_tables.iter().enumerate() {
            for y in 0..pt.pages_y {
                for x in 0..pt.pages_x {
                    let flat = bit_offset + y * pt.pages_x + x;
                    let word = (flat / 32) as usize;
                    let bit = flat % 32;
                    let requested = word < requested_bits.len()
                        && (requested_bits[word] & (1 << bit)) != 0;

                    if requested {
                        let entry = pt.get(x, y);
                        if entry & PAGE_VALID_BIT != 0 && entry & PAGE_CACHED_BIT == 0 {
                            let phys = (entry & PAGE_PHYS_MASK) as u16;
                            dirty.push((level as u32, x, y, phys));
                        }
                    }
                }
            }
            bit_offset += pt.pages_x * pt.pages_y;
        }
        dirty
    }
}
