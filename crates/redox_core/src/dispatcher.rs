//! Dispatcher and stage management for organizing system execution order.

use std::collections::HashMap;

use redox_ecs::World;

use crate::time::Time;

/// Stages defining the execution order of systems.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Stage {
    /// Reading input states or window events.
    Input,
    /// Main frame update logic.
    Update,
    /// Fixed-step updates (e.g. physics steps).
    PhysicsSync,
    /// Logic executed after physics and standard updates.
    PostUpdate,
    /// Preparing data to send to the GPU (collecting models, extracting transforms, etc).
    RenderPrep,
    /// Issuing draw commands to the renderer.
    Render,
}

/// A dispatcher that linearly executes systems registered to different stages.
/// 
/// Generic over `Context` (usually `RenderContext`).
pub struct Dispatcher<Context> {
    systems: HashMap<Stage, Vec<Box<dyn FnMut(&mut World, &mut Context)>>>,
}

impl<Context> Dispatcher<Context> {
    /// Initializes an empty dispatcher.
    pub fn new() -> Self {
        Self {
            systems: HashMap::new(),
        }
    }

    /// Registers a system to the given stage.
    pub fn add_system<F>(&mut self, stage: Stage, system: F)
    where
        F: FnMut(&mut World, &mut Context) + 'static,
    {
        self.systems
            .entry(stage)
            .or_insert_with(Vec::new)
            .push(Box::new(system));
    }

    /// Runs all registered systems in order.
    pub fn run(&mut self, world: &mut World, context: &mut Context, time: &mut Time) {
        let stages_in_order = [
            Stage::Input,
            Stage::Update,
            Stage::PhysicsSync,
            Stage::PostUpdate,
            Stage::RenderPrep,
            Stage::Render,
        ];

        for stage in stages_in_order {
            if let Some(systems) = self.systems.get_mut(&stage) {
                if stage == Stage::PhysicsSync {
                    // Execute physics/fixed updates multiple times if needed to catch up
                    while time.should_step_fixed() {
                        for system in systems.iter_mut() {
                            system(world, context);
                        }
                        time.consume_fixed_step();
                    }
                } else {
                    // Standard single execution per frame
                    for system in systems.iter_mut() {
                        system(world, context);
                    }
                }
            }
        }
    }
}

impl<Context> Default for Dispatcher<Context> {
    fn default() -> Self {
        Self::new()
    }
}
