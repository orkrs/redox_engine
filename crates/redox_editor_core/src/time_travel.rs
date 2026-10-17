use redox_reflect::{TypeRegistry, WorldSnapshot};
use std::collections::VecDeque;

/// Ring buffer of world snapshots for scrubbing back through recent frames.
pub struct TimeTravelBuffer {
    /// Stored snapshots, oldest first.
    pub frames: VecDeque<WorldSnapshot>,
    /// Maximum number of snapshots kept.
    pub capacity: usize,
}

impl Default for TimeTravelBuffer {
    fn default() -> Self {
        Self {
            frames: VecDeque::new(),
            capacity: 1800,
        }
    }
}

impl TimeTravelBuffer {
    /// Creates a buffer holding up to `capacity` snapshots.
    pub fn new(capacity: usize) -> Self {
        Self {
            frames: VecDeque::with_capacity(capacity),
            capacity,
        }
    }
    /// Appends a snapshot, dropping the oldest when full.
    pub fn push(&mut self, snapshot: WorldSnapshot) {
        if self.frames.len() >= self.capacity {
            self.frames.pop_front();
        }
        self.frames.push_back(snapshot);
    }
    /// Number of stored snapshots.
    pub fn len(&self) -> usize {
        self.frames.len()
    }
    /// Whether no snapshot is stored.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
    /// Snapshot at `idx` (0 is the oldest).
    pub fn get(&self, idx: usize) -> Option<&WorldSnapshot> {
        self.frames.get(idx)
    }
    /// Restores the world to the snapshot at `idx`.
    pub fn restore(&self, world: &mut redox_ecs::World, registry: &TypeRegistry, idx: usize) {
        if let Some(snap) = self.get(idx) {
            let entities: Vec<_> = world.all_entities().collect();
            for e in entities {
                world.despawn(e);
            }
            for ent_snap in &snap.entities {
                let e = world.spawn();
                for comp in &ent_snap.components {
                    let _ = registry.insert_component(world, e, comp);
                }
            }
        }
    }
}
