use std::sync::atomic::{AtomicU32, Ordering};
use crossbeam_queue::SegQueue;
use parking_lot::RwLock;

/// A unique identifier for an entity.
///
/// Combines an index and a generation to detect stale references.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Entity {
    pub(crate) id: u32,
    pub(crate) generation: u32,
}

impl Entity {
    /// Returns the raw index of the entity.
    #[inline]
    pub fn id(&self) -> u32 { self.id }

    /// Returns the generation of the entity.
    #[inline]
    pub fn generation(&self) -> u32 { self.generation }
}

/// Allocator for entity IDs with generation counters.
///
/// Uses a lock-free queue for free IDs and a generation vector protected by `RwLock`.
pub struct EntityAllocator {
    generations: RwLock<Vec<u32>>,
    free_ids: SegQueue<u32>,
    next_id: AtomicU32,
}

impl EntityAllocator {
    /// Creates a new entity allocator.
    pub fn new() -> Self {
        Self {
            generations: RwLock::new(Vec::with_capacity(1024)),
            free_ids: SegQueue::new(),
            next_id: AtomicU32::new(0),
        }
    }

    /// Allocates a new unique entity.
    pub fn allocate(&self) -> Entity {
        let id = if let Some(free_id) = self.free_ids.pop() {
            free_id
        } else {
            self.next_id.fetch_add(1, Ordering::Relaxed)
        };

        // Ensure the generation vector is large enough
        {
            let gens = self.generations.read();
            if id as usize >= gens.len() {
                drop(gens); // Release read lock before acquiring write lock
                let mut gens_mut = self.generations.write();
                if id as usize >= gens_mut.len() {
                    gens_mut.resize((id as usize) + 1, 0);
                }
            }
        }

        let generation = {
            let gens = self.generations.read();
            gens[id as usize]
        };

        Entity { id, generation }
    }

    /// Deallocates an entity, allowing its ID to be reused.
    ///
    /// Returns `true` if the entity was valid and deallocated, `false` otherwise.
    pub fn deallocate(&self, entity: Entity) -> bool {
        let mut gens = self.generations.write();
        if entity.id as usize >= gens.len() {
            return false;
        }

        let current_gen = gens[entity.id as usize];
        if current_gen != entity.generation {
            return false; // Stale handle
        }

        // Increment generation to invalidate old handles
        gens[entity.id as usize] += 1;
        self.free_ids.push(entity.id);
        true
    }


}

impl Default for EntityAllocator {
    fn default() -> Self { Self::new() }
}