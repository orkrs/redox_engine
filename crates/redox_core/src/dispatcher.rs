//! Dispatcher and stage management for organizing system execution order.
//!
//! Sequential execution remains the default via [`Dispatcher::add_system`].
//! Parallel systems within a stage declare resource access via [`SystemMeta`];
//! non-conflicting writers are batched and executed with `rayon`.

use std::any::TypeId;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use rayon::prelude::*;
use redox_ecs::World;

use crate::time::Time;

/// Maximum fixed steps executed per frame before the accumulator is reset.
///
/// Prevents the spiral of death: a slow frame cannot queue unbounded physics
/// work that makes the next frame slower still.
pub const MAX_FIXED_STEPS: u32 = 5;

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

/// Declared read/write resource access for parallel scheduling (MVP: type-id sets).
#[derive(Clone, Debug, Default)]
pub struct SystemMeta {
    pub reads: Vec<TypeId>,
    pub writes: Vec<TypeId>,
    /// When true and access does not conflict, the system may run in parallel.
    pub parallel: bool,
}

impl SystemMeta {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reads(mut self, ids: &[TypeId]) -> Self {
        self.reads.extend_from_slice(ids);
        self
    }

    pub fn writes(mut self, ids: &[TypeId]) -> Self {
        self.writes.extend_from_slice(ids);
        self
    }

    pub fn parallel(mut self, parallel: bool) -> Self {
        self.parallel = parallel;
        self
    }

    /// Exclusive write of a single resource type (Physics / Audio / AI DAG nodes).
    pub fn write_resource<T: 'static>() -> Self {
        Self::new().writes(&[TypeId::of::<T>()]).parallel(true)
    }

    pub fn read_resource<T: 'static>() -> Self {
        Self::new().reads(&[TypeId::of::<T>()]).parallel(true)
    }
}

enum SystemSlot<Context> {
    Sequential(Box<dyn FnMut(&mut World, &mut Context)>),
    Parallel {
        meta: SystemMeta,
        func: Arc<dyn Fn(&World, &Context) + Send + Sync>,
    },
}

/// A dispatcher that executes systems registered to different stages.
///
/// Generic over `Context` (usually `RenderContext`).
pub struct Dispatcher<Context> {
    systems: HashMap<Stage, Vec<SystemSlot<Context>>>,
    /// Per-frame maintenance hooks (e.g. `Events::update` for registered event types).
    frame_end_hooks: Vec<Box<dyn FnMut(&mut World)>>,
}

/// Returns true when two parallel systems may run concurrently.
pub fn access_compatible(a: &SystemMeta, b: &SystemMeta) -> bool {
    let a_writes: HashSet<TypeId> = a.writes.iter().copied().collect();
    let b_writes: HashSet<TypeId> = b.writes.iter().copied().collect();
    if !a_writes.is_disjoint(&b_writes) {
        return false;
    }
    // Writer vs reader conflict.
    let a_reads: HashSet<TypeId> = a.reads.iter().copied().collect();
    let b_reads: HashSet<TypeId> = b.reads.iter().copied().collect();
    a_writes.is_disjoint(&b_reads) && b_writes.is_disjoint(&a_reads)
}

/// Topological layers of parallel systems within one stage (each layer runs in parallel).
pub fn parallel_layers(metas: &[SystemMeta]) -> Vec<Vec<usize>> {
    let n = metas.len();
    if n == 0 {
        return Vec::new();
    }

    let mut indegree = vec![0usize; n];
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];

    for i in 0..n {
        for j in (i + 1)..n {
            if !access_compatible(&metas[i], &metas[j]) {
                // i must finish before j when they conflict (stable registration order).
                edges[i].push(j);
                indegree[j] += 1;
            }
        }
    }

    let mut layers = Vec::new();
    let mut ready: VecDeque<usize> = (0..n).filter(|&i| indegree[i] == 0).collect();
    let mut processed = 0usize;

    while !ready.is_empty() {
        let mut layer = Vec::new();
        let batch_len = ready.len();
        for _ in 0..batch_len {
            let idx = ready.pop_front().expect("ready queue");
            layer.push(idx);
            processed += 1;
            for &next in &edges[idx] {
                indegree[next] -= 1;
                if indegree[next] == 0 {
                    ready.push_back(next);
                }
            }
        }
        layers.push(layer);
    }

    if processed != n {
        // Cycle or unresolved conflict — fall back to sequential layers of size 1.
        return (0..n).map(|i| vec![i]).collect();
    }

    layers
}

impl<Context> Dispatcher<Context>
where
    Context: Send + Sync,
{
    /// Initializes an empty dispatcher.
    pub fn new() -> Self {
        Self {
            systems: HashMap::new(),
            frame_end_hooks: Vec::new(),
        }
    }

    /// Registers a sequential system (default, preserves existing behaviour).
    pub fn add_system<F>(&mut self, stage: Stage, system: F)
    where
        F: FnMut(&mut World, &mut Context) + 'static,
    {
        self.systems
            .entry(stage)
            .or_default()
            .push(SystemSlot::Sequential(Box::new(system)));
    }

    /// Registers a parallel-safe system with explicit access metadata.
    ///
    /// Parallel systems receive `&World` and `&Context` (immutable). Mutating
    /// ECS state requires the sequential [`Self::add_system`] path.
    pub fn add_system_parallel<F>(&mut self, stage: Stage, meta: SystemMeta, system: F)
    where
        F: Fn(&World, &Context) + Send + Sync + 'static,
    {
        self.systems
            .entry(stage)
            .or_default()
            .push(SystemSlot::Parallel {
                meta,
                func: Arc::new(system),
            });
    }

    /// Register several independent sim systems (Physics / Audio / AI) on
    /// [`Stage::Update`]. Non-overlapping `writes` run in the same rayon layer.
    pub fn add_sim_systems<I, F>(&mut self, systems: I)
    where
        I: IntoIterator<Item = (SystemMeta, F)>,
        F: Fn(&World, &Context) + Send + Sync + 'static,
    {
        for (meta, func) in systems {
            self.add_system_parallel(Stage::Update, meta.parallel(true), func);
        }
    }

    /// Registers a hook called once at the end of every frame.
    pub fn add_frame_end_hook<F>(&mut self, hook: F)
    where
        F: FnMut(&mut World) + 'static,
    {
        self.frame_end_hooks.push(Box::new(hook));
    }

    fn run_stage_parallel(
        slots: &mut [SystemSlot<Context>],
        world: &mut World,
        context: &mut Context,
    ) {
        let mut parallel_batch: Vec<(SystemMeta, usize)> = Vec::new();

        for idx in 0..slots.len() {
            let is_sequential = matches!(slots[idx], SystemSlot::Sequential(_));
            if is_sequential {
                if !parallel_batch.is_empty() {
                    Self::flush_parallel_batch(&mut parallel_batch, &slots, world, context);
                }
                if let SystemSlot::Sequential(func) = &mut slots[idx] {
                    func(world, context);
                }
                continue;
            }

            let run_parallel = matches!(
                &slots[idx],
                SystemSlot::Parallel { meta, .. } if meta.parallel
            );
            if run_parallel {
                if let SystemSlot::Parallel { meta, .. } = &slots[idx] {
                    parallel_batch.push((meta.clone(), idx));
                }
            } else {
                if !parallel_batch.is_empty() {
                    Self::flush_parallel_batch(&mut parallel_batch, &slots, world, context);
                }
                if let SystemSlot::Parallel { func, .. } = &slots[idx] {
                    func(world, &*context);
                }
            }
        }

        if !parallel_batch.is_empty() {
            Self::flush_parallel_batch(&mut parallel_batch, &slots, world, context);
        }
    }

    fn flush_parallel_batch(
        batch: &mut Vec<(SystemMeta, usize)>,
        slots: &[SystemSlot<Context>],
        world: &World,
        context: &Context,
    ) {
        let metas: Vec<SystemMeta> = batch.iter().map(|(m, _)| m.clone()).collect();
        let indices: Vec<usize> = batch.iter().map(|(_, i)| *i).collect();
        batch.clear();

        for layer in parallel_layers(&metas) {
            let funcs: Vec<Arc<dyn Fn(&World, &Context) + Send + Sync>> = layer
                .iter()
                .map(|&layer_pos| {
                    let slot_idx = indices[layer_pos];
                    match &slots[slot_idx] {
                        SystemSlot::Parallel { func, .. } => Arc::clone(func),
                        SystemSlot::Sequential(_) => unreachable!("parallel batch slot"),
                    }
                })
                .collect();
            funcs.par_iter().for_each(|func| func(world, context));
        }
    }

    /// Runs all registered systems in order.
    pub fn run(&mut self, world: &mut World, context: &mut Context) {
        world.begin_frame();

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
                    let mut steps = 0u32;
                    while steps < MAX_FIXED_STEPS {
                        let should_step = world
                            .get_resource::<Time>()
                            .is_some_and(|t| t.should_step_fixed());
                        if !should_step {
                            break;
                        }
                        Self::run_stage_parallel(systems, world, context);
                        world
                            .get_resource_mut::<Time>()
                            .expect("Time resource removed mid-frame")
                            .consume_fixed_step();
                        steps += 1;
                    }
                    if steps == MAX_FIXED_STEPS {
                        log::warn!("Fixed-step cap ({MAX_FIXED_STEPS}) hit; resetting accumulator");
                        world
                            .get_resource_mut::<Time>()
                            .expect("Time resource removed mid-frame")
                            .reset_accumulator();
                    } else if let Some(time) = world.get_resource_mut::<Time>() {
                        time.update_alpha();
                    }
                } else {
                    Self::run_stage_parallel(systems, world, context);
                }
            }
        }

        for hook in self.frame_end_hooks.iter_mut() {
            hook(world);
        }
    }
}

impl<Context> Default for Dispatcher<Context>
where
    Context: Send + Sync,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_compatible_disjoint_writes() {
        let a = SystemMeta::default().writes(&[TypeId::of::<u32>()]);
        let b = SystemMeta::default().writes(&[TypeId::of::<i32>()]);
        assert!(access_compatible(&a, &b));
    }

    #[test]
    fn access_compatible_rejects_write_write_overlap() {
        let a = SystemMeta::default().writes(&[TypeId::of::<u32>()]);
        let b = SystemMeta::default().writes(&[TypeId::of::<u32>()]);
        assert!(!access_compatible(&a, &b));
    }

    #[test]
    fn access_compatible_rejects_write_read_overlap() {
        let writer = SystemMeta::default().writes(&[TypeId::of::<u32>()]);
        let reader = SystemMeta::default().reads(&[TypeId::of::<u32>()]);
        assert!(!access_compatible(&writer, &reader));
    }

    #[test]
    fn parallel_layers_orders_conflicts() {
        let metas = vec![
            SystemMeta::default().writes(&[TypeId::of::<u32>()]).parallel(true),
            SystemMeta::default().writes(&[TypeId::of::<u32>()]).parallel(true),
            SystemMeta::default().writes(&[TypeId::of::<i32>()]).parallel(true),
        ];
        let layers = parallel_layers(&metas);
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0], vec![0, 2]);
        assert_eq!(layers[1], vec![1]);
    }

    #[test]
    fn parallel_layers_independent_systems_single_layer() {
        let metas = vec![
            SystemMeta::default().writes(&[TypeId::of::<u32>()]).parallel(true),
            SystemMeta::default().writes(&[TypeId::of::<i32>()]).parallel(true),
            SystemMeta::default().writes(&[TypeId::of::<f32>()]).parallel(true),
        ];
        let layers = parallel_layers(&metas);
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].len(), 3);
    }

    #[test]
    fn parallel_layers_reader_waits_for_writer() {
        let metas = vec![
            SystemMeta::default().writes(&[TypeId::of::<u32>()]).parallel(true),
            SystemMeta::default().writes(&[TypeId::of::<i32>()]).parallel(true),
            SystemMeta::default()
                .reads(&[TypeId::of::<u32>()])
                .parallel(true),
        ];
        let layers = parallel_layers(&metas);
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0], vec![0, 1]);
        assert_eq!(layers[1], vec![2]);
    }
}
