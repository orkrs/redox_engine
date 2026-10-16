//! Virtual→physical page translation for Virtual Shadow Maps (VSM).
//!
//! Each clipmap level owns one [`VirtualPageTable`] mapping its virtual
//! page grid to physical pages in the shared shadow atlas. An entry word
//! packs a valid bit, a cached bit and the physical page index:
//!
//! ```text
//! bit 31      bit 30        bits 29..0
//! VALID       CACHED        physical page index (PAGE_PHYS_MASK)
//! ```
//!
//! The words are mirrored 1:1 to the GPU (storage buffer / texture upload),
//! so the layout is part of the shader contract — see `pbr.wgsl::VsmGpuInfo`
//! and `shaders/vsm_visibility.wgsl`.

/// Entry has been assigned a physical page.
pub const PAGE_VALID_BIT: u32 = 1 << 31;
/// Entry's physical page holds up-to-date shadow data.
pub const PAGE_CACHED_BIT: u32 = 1 << 30;
/// Mask extracting the physical page index from an entry word.
pub const PAGE_PHYS_MASK: u32 = 0x3FFF_FFFF;

/// Extracts the physical page index from an entry word.
#[inline]
pub fn phys_page(entry: u32) -> u16 {
    (entry & PAGE_PHYS_MASK) as u16
}

/// Virtual page grid of one clipmap level (or one face of a local light).
///
/// Row-major `entries[y * pages_x + x]`, mirroring the flat indexing used
/// by the GPU visibility pass (`page_table_offset + y * pages_per_side + x`).
pub struct VirtualPageTable {
    /// Pages along X.
    pub pages_x: u32,
    /// Pages along Y.
    pub pages_y: u32,
    /// Raw entry words, exposed for flat GPU upload.
    pub entries: Vec<u32>,
}

impl VirtualPageTable {
    /// Creates a table where every page is invalid.
    pub fn new(pages_x: u32, pages_y: u32) -> Self {
        Self {
            pages_x,
            pages_y,
            entries: vec![0u32; (pages_x * pages_y) as usize],
        }
    }

    /// Number of entries (== `pages_x * pages_y`).
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the grid has no cells.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Raw entry at `(x, y)` (out-of-range reads yield `0`, i.e. invalid).
    #[inline]
    pub fn get(&self, x: u32, y: u32) -> u32 {
        if x >= self.pages_x || y >= self.pages_y {
            return 0;
        }
        self.entries[(y * self.pages_x + x) as usize]
    }

    /// Overwrites the entry at `(x, y)` (no-op when out of range).
    #[inline]
    pub fn set(&mut self, x: u32, y: u32, val: u32) {
        if x < self.pages_x && y < self.pages_y {
            self.entries[(y * self.pages_x + x) as usize] = val;
        }
    }

    /// Marks every page invalid **without** returning physical pages to any
    /// allocator — pair with a walk that frees `phys_page(entry)` first when
    /// the pages live in a shared pool (see `VsmClipmap::invalidate_all`).
    #[inline]
    pub fn invalidate_all(&mut self) {
        self.entries.fill(0);
    }

    /// Sets the CACHED bit on a valid entry (after its page was rendered).
    #[inline]
    pub fn mark_cached(&mut self, x: u32, y: u32) {
        let entry = self.get(x, y);
        if entry & PAGE_VALID_BIT != 0 {
            self.set(x, y, entry | PAGE_CACHED_BIT);
        }
    }

    /// Clears the CACHED bit on a valid entry (dirtying the page).
    #[inline]
    pub fn clear_cached(&mut self, x: u32, y: u32) {
        let entry = self.get(x, y);
        if entry & PAGE_VALID_BIT != 0 {
            self.set(x, y, entry & !PAGE_CACHED_BIT);
        }
    }
}

/// Free-list allocator for physical atlas pages.
///
/// Pages handed out by [`alloc`](PhysPageAllocator::alloc) must eventually
/// come back through [`free`](PhysPageAllocator::free) — either explicitly
/// or by walking page tables before invalidation.
pub struct PhysPageAllocator {
    free: Vec<u16>,
    /// Total pages managed (allocated + free); kept for diagnostics.
    total: usize,
}

impl PhysPageAllocator {
    /// Creates a pool with every page in `0..total_pages` unallocated.
    ///
    /// Indices are popped lowest-first so early allocations land at the
    /// top-left of the atlas, keeping the used region dense.
    pub fn new(total_pages: u32) -> Self {
        let total_pages = total_pages.min(u16::MAX as u32) as usize;
        Self {
            free: (0..total_pages as u16).rev().collect(),
            total: total_pages,
        }
    }

    /// Number of pages the pool manages in total.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.total
    }

    /// Number of currently free pages.
    #[inline]
    pub fn free_count(&self) -> usize {
        self.free.len()
    }

    /// Takes one physical page, or `None` when the pool is exhausted.
    #[inline]
    pub fn alloc(&mut self) -> Option<u16> {
        self.free.pop()
    }

    /// Returns a page to the pool. Double frees are caught in debug builds
    /// and ignored otherwise (the pool stays consistent either way).
    pub fn free(&mut self, idx: u16) {
        if !self.free.contains(&idx) {
            self.free.push(idx);
        } else {
            debug_assert!(false, "PhysPageAllocator: page {idx} freed twice");
        }
    }
}
impl Default for PhysPageAllocator {
    fn default() -> Self {
        Self::new(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_exhausts_and_wraps() {
        let mut pool = PhysPageAllocator::new(4);
        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.push(pool.alloc().expect("pool not exhausted"));
        }
        assert!(pool.alloc().is_none(), "pool should be exhausted");
        assert_eq!(seen.len(), 4);
        assert!(seen.iter().collect::<std::collections::HashSet<_>>().len() == 4);
        assert_eq!(pool.free_count(), 0);

        pool.free(seen[0]);
        assert_eq!(pool.alloc(), Some(seen[0]));
        assert!(pool.alloc().is_none());
    }

    #[test]
    fn free_makes_page_reusable() {
        let mut pool = PhysPageAllocator::new(2);
        let a = pool.alloc().unwrap();
        let _b = pool.alloc().unwrap();
        assert!(pool.alloc().is_none());

        pool.free(a);
        assert_eq!(pool.free_count(), 1);
        assert_eq!(pool.alloc(), Some(a));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "freed twice")]
    fn double_free_panics_in_debug() {
        let mut pool = PhysPageAllocator::new(2);
        let a = pool.alloc().unwrap();
        pool.free(a);
        pool.free(a);
    }

    #[test]
    fn table_get_set_invalidate_roundtrip() {
        let mut pt = VirtualPageTable::new(16, 16);
        assert_eq!(pt.len(), 256);
        assert!(pt.entries.iter().all(|&e| e == 0));

        pt.set(3, 5, 0x1234 | PAGE_VALID_BIT | PAGE_CACHED_BIT);
        assert_eq!(pt.get(3, 5), 0x1234 | PAGE_VALID_BIT | PAGE_CACHED_BIT);
        assert_eq!(phys_page(pt.get(3, 5)), 0x1234);
        // Out-of-range accesses are safe no-ops / zeros.
        assert_eq!(pt.get(99, 99), 0);
        pt.set(99, 99, 0xFFFF_FFFF);
        assert_eq!(pt.len(), 256);

        pt.clear_cached(3, 5);
        assert_eq!(pt.get(3, 5) & PAGE_CACHED_BIT, 0);
        pt.mark_cached(3, 5);
        assert_ne!(pt.get(3, 5) & PAGE_CACHED_BIT, 0);

        pt.invalidate_all();
        assert!(pt.entries.iter().all(|&e| e == 0));
    }
}
