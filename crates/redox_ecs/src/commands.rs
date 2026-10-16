//! Deferred world mutations.
//!
//! Systems that iterate a query cannot also spawn or despawn entities. They record those
//! changes in a [`CommandQueue`] through [`Commands`] and the owner applies the queue at a
//! synchronisation point with [`CommandQueue::apply`].
//!
//! Entity ids for `spawn` are reserved immediately (the allocator is lock-free and takes
//! `&self`), so the returned [`Entity`] can be referenced by later commands in the same queue.

use crate::Component;
use crate::entity::{Entity, EntityAllocator};
use crate::world::{Bundle, World};

type Command = Box<dyn FnOnce(&mut World) + Send>;

/// Ordered list of recorded world mutations.
#[derive(Default)]
pub struct CommandQueue {
    ops: Vec<Command>,
}

impl CommandQueue {
    /// Creates an empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of pending commands.
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// `true` when nothing is pending.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Runs every command in recording order and empties the queue.
    pub fn apply(&mut self, world: &mut World) {
        for op in self.ops.drain(..) {
            op(world);
        }
    }
}

/// Recording handle: borrows the world's id allocator and a queue.
pub struct Commands<'a> {
    entities: &'a EntityAllocator,
    queue: &'a mut CommandQueue,
}

impl<'a> Commands<'a> {
    /// Creates a handle that records into `queue` and reserves ids from `world`.
    pub fn new(world: &'a World, queue: &'a mut CommandQueue) -> Self {
        Self {
            entities: world.entity_allocator(),
            queue,
        }
    }

    fn push(&mut self, op: impl FnOnce(&mut World) + Send + 'static) {
        self.queue.ops.push(Box::new(op));
    }

    /// Reserves an entity and spawns it with `bundle` when the queue is applied.
    pub fn spawn<B: Bundle + Send + 'static>(&mut self, bundle: B) -> Entity {
        let entity = self.entities.allocate();
        self.push(move |world| world.spawn_bundle_at(entity, bundle));
        entity
    }

    /// Despawns `entity` when the queue is applied (no-op for stale handles).
    pub fn despawn(&mut self, entity: Entity) {
        self.push(move |world| world.despawn(entity));
    }

    /// Adds or overwrites a component when the queue is applied.
    pub fn insert<T: Component>(&mut self, entity: Entity, component: T) {
        self.push(move |world| world.add_component(entity, component));
    }

    /// Removes a component when the queue is applied.
    pub fn remove<T: Component>(&mut self, entity: Entity) {
        self.push(move |world| {
            world.remove_component::<T>(entity);
        });
    }

    /// Inserts or replaces a resource when the queue is applied.
    pub fn insert_resource<R: 'static + Send + Sync>(&mut self, resource: R) {
        self.push(move |world| world.insert_resource(resource));
    }

    /// Records an arbitrary mutation.
    pub fn add(&mut self, op: impl FnOnce(&mut World) + Send + 'static) {
        self.push(op);
    }
}
