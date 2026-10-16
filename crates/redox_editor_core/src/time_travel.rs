use std::collections::VecDeque;
use redox_reflect::{TypeRegistry, WorldSnapshot};

pub struct TimeTravelBuffer {
    pub frames: VecDeque<WorldSnapshot>,
    pub capacity: usize,
}

impl Default for TimeTravelBuffer {
    fn default() -> Self { Self { frames: VecDeque::new(), capacity: 1800 } }
}

impl TimeTravelBuffer {
    pub fn new(capacity: usize) -> Self { Self { frames: VecDeque::with_capacity(capacity), capacity } }
    pub fn push(&mut self, snapshot: WorldSnapshot) {
        if self.frames.len() >= self.capacity { self.frames.pop_front(); }
        self.frames.push_back(snapshot);
    }
    pub fn len(&self) -> usize { self.frames.len() }
    pub fn is_empty(&self) -> bool { self.frames.is_empty() }
    pub fn get(&self, idx: usize) -> Option<&WorldSnapshot> { self.frames.get(idx) }
    pub fn restore(&self, world: &mut redox_ecs::World, registry: &TypeRegistry, idx: usize) {
        if let Some(snap) = self.get(idx) {
            let entities: Vec<_> = world.all_entities().collect();
            for e in entities { world.despawn(e); }
            for ent_snap in &snap.entities {
                let e = world.spawn();
                for comp in &ent_snap.components { let _ = registry.insert_component(world, e, comp); }
            }
        }
    }
}
