//! Fixed-size pool of streamed pages with least-recently-used eviction
//! (VG-03 step 3), plus the pure request filter `pages_to_load` (step 4).
//!
//! Resident (always loaded) pages are not part of the pool.

use std::collections::{HashMap, HashSet};

/// Occupied slot of the pool.
#[derive(Clone, Copy, Debug)]
struct Slot {
    page: u32,
    /// Frame in which the page was last needed.
    last_used: u64,
}

/// A fixed number of page slots. A page touched in the current frame is never
/// evicted, so a frame can always use every page it requested.
#[derive(Debug)]
pub struct PagePool {
    slots: Vec<Option<Slot>>,
    /// Page id -> slot index, for resident pages only.
    index: HashMap<u32, usize>,
}

impl PagePool {
    /// Creates an empty pool with `capacity_pages` slots.
    pub fn new(capacity_pages: u32) -> Self {
        let capacity = usize::try_from(capacity_pages).unwrap_or(usize::MAX);
        Self {
            slots: vec![None; capacity],
            index: HashMap::new(),
        }
    }

    /// Marks `page` as needed in `frame`. Returns its slot when it is resident;
    /// returns `None` for a page that is not resident (nothing is recorded).
    pub fn touch(&mut self, page: u32, frame: u64) -> Option<u32> {
        let slot = *self.index.get(&page)?;
        let entry = self.slots.get_mut(slot)?.as_mut()?;
        entry.last_used = frame;
        u32::try_from(slot).ok()
    }

    /// Gives `page` a slot in `frame`. Uses a free slot if there is one, otherwise
    /// evicts the least recently used page that was not touched in `frame`.
    ///
    /// Returns `(slot, evicted_page)`. Returns `None` when every slot is in use
    /// in this frame. An already resident page is simply touched.
    pub fn allocate(&mut self, page: u32, frame: u64) -> Option<(u32, Option<u32>)> {
        if let Some(slot) = self.touch(page, frame) {
            return Some((slot, None));
        }
        let (slot, evicted) = match self.slots.iter().position(Option::is_none) {
            Some(free) => (free, None),
            None => {
                let (victim_slot, victim_page) = self.least_recently_used(frame)?;
                (victim_slot, Some(victim_page))
            }
        };
        if let Some(old) = evicted {
            self.index.remove(&old);
        }
        *self.slots.get_mut(slot)? = Some(Slot {
            page,
            last_used: frame,
        });
        self.index.insert(page, slot);
        Some((u32::try_from(slot).ok()?, evicted))
    }

    /// True when `page` currently holds a slot.
    pub fn is_resident(&self, page: u32) -> bool {
        self.index.contains_key(&page)
    }

    /// Number of pages currently holding a slot.
    pub fn resident_count(&self) -> usize {
        self.index.len()
    }

    /// Slot and page of the occupied slot with the smallest `last_used` among
    /// those not used in `frame`. Ties go to the lower slot index.
    fn least_recently_used(&self, frame: u64) -> Option<(usize, u32)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(slot, entry)| {
                let entry = entry.as_ref()?;
                (entry.last_used < frame).then_some((slot, entry.last_used, entry.page))
            })
            .min_by_key(|&(slot, last_used, _)| (last_used, slot))
            .map(|(slot, _, page)| (slot, page))
    }
}

/// Chooses which requested pages to load this frame.
///
/// Keeps the requests in their given order, drops pages that are already
/// resident in `pool` and duplicates (first occurrence wins), and returns at
/// most `budget_pages` pages. The caller orders `requests` by priority.
pub fn pages_to_load(requests: &[u32], pool: &PagePool, budget_pages: usize) -> Vec<u32> {
    let mut seen: HashSet<u32> = HashSet::new();
    let mut out = Vec::new();
    for &page in requests {
        if out.len() >= budget_pages {
            break;
        }
        if pool.is_resident(page) || !seen.insert(page) {
            continue;
        }
        out.push(page);
    }
    out
}
