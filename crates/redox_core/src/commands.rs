//! Deferred world mutations for systems, applied by the dispatcher between stages.
//!
//! Systems that iterate a query cannot spawn or despawn at the same time. They record the
//! changes through [`SharedCommands::record`] and the dispatcher applies the queue after
//! every stage (and after every fixed step), see [`flush_commands`].

use std::sync::{Mutex, PoisonError};

use redox_ecs::{CommandQueue, Commands, World};

/// World-global command queue (resource); `Sync`, so shared-access systems can record.
#[derive(Default)]
pub struct SharedCommands(Mutex<CommandQueue>);

impl SharedCommands {
    /// Creates an empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records commands with `f`. Entity ids from `Commands::spawn` are valid immediately.
    pub fn record<R>(&self, world: &World, f: impl FnOnce(&mut Commands<'_>) -> R) -> R {
        let mut queue = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let mut commands = Commands::new(world, &mut queue);
        f(&mut commands)
    }

    /// Number of pending commands.
    pub fn len(&self) -> usize {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).len()
    }

    /// `true` when nothing is pending.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn take(&self) -> CommandQueue {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// Applies the world's [`SharedCommands`] queue; no-op when the resource is absent or empty.
/// Commands recorded by commands (via hooks) wait for the next flush.
pub fn flush_commands(world: &mut World) {
    let Some(shared) = world.get_resource::<SharedCommands>() else {
        return;
    };
    if shared.is_empty() {
        return;
    }
    let mut queue = shared.take();
    queue.apply(world);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flush_applies_recorded_spawns_and_empties_the_queue() {
        let mut world = World::new();
        world.insert_resource(SharedCommands::new());
        let entity = {
            let shared = world.get_resource::<SharedCommands>().unwrap();
            shared.record(&world, |cmds| cmds.spawn((5u32,)))
        };
        assert!(world.get_component::<u32>(entity).is_none(), "deferred");
        flush_commands(&mut world);
        assert_eq!(world.get_component::<u32>(entity), Some(&5));
        assert!(world.get_resource::<SharedCommands>().unwrap().is_empty());
    }

    #[test]
    fn flush_without_the_resource_is_a_no_op() {
        flush_commands(&mut World::new());
    }

    #[test]
    fn dispatcher_flushes_between_stages() {
        use crate::dispatcher::{Dispatcher, Stage};

        let mut world = World::new();
        world.insert_resource(SharedCommands::new());
        let mut dispatcher = Dispatcher::<()>::new();
        dispatcher.add_system(Stage::Update, |world, _| {
            let shared = world.get_resource::<SharedCommands>().unwrap();
            shared.record(world, |cmds| cmds.spawn((1u8,)));
        });
        dispatcher.add_system(Stage::PostUpdate, |world, _| {
            let seen = world.query::<&u8>().iter(world).count();
            world.insert_resource(seen);
        });
        dispatcher.run(&mut world, &mut ());
        assert_eq!(world.get_resource::<usize>(), Some(&1));
    }
}
