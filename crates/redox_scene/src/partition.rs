//! Grid world-partition with hysteresis, per-tick budget, and ECS spawn/despawn.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use redox_ecs::query::Query;
use redox_ecs::world::World;
use redox_ecs::{Changed, Entity};
use redox_math::{Quat, Transform, Vec3};

use crate::ofpa::{
    ActorRecord, WorldManifest, cell_mtime, load_cell_actors, load_manifest, save_actor,
};
use crate::{PartitionCell, SceneResult};

/// Marker on every entity spawned from a streamed cell.
#[derive(Clone, Debug)]
pub struct StreamedActor {
    pub cell: PartitionCell,
    pub id: String,
    pub kind: String,
}

#[derive(Clone, Copy, Debug)]
pub struct StreamingBudget {
    pub max_loads: usize,
    pub max_unloads: usize,
}

impl Default for StreamingBudget {
    fn default() -> Self {
        Self {
            max_loads: 2,
            max_unloads: 4,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct StreamingConfig {
    /// Chebyshev radius (in cells) that must be resident.
    pub load_radius: i32,
    /// Unload only outside this radius (`>= load_radius`).
    pub unload_radius: i32,
    pub budget: StreamingBudget,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            load_radius: 1,
            unload_radius: 2,
            budget: StreamingBudget::default(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct StreamingReport {
    pub loaded: Vec<PartitionCell>,
    pub unloaded: Vec<PartitionCell>,
    pub reloaded: Vec<PartitionCell>,
    pub written: usize,
}

struct LoadedCell {
    entities: Vec<Entity>,
    mtime: Option<SystemTime>,
}

/// Runtime streamer. Call [`WorldPartition::tick`] (or [`tick_streaming`]) each frame.
pub struct WorldPartition {
    pub root: PathBuf,
    pub manifest: WorldManifest,
    pub config: StreamingConfig,
    catalog: HashSet<PartitionCell>,
    loaded: HashMap<PartitionCell, LoadedCell>,
    /// In-memory cells used when no OFPA tree is present (tests / generated).
    memory: HashMap<PartitionCell, Vec<ActorRecord>>,
}

impl WorldPartition {
    pub fn open(root: impl AsRef<Path>) -> SceneResult<Self> {
        let root = root.as_ref().to_path_buf();
        let manifest = load_manifest(&root)?;
        let catalog = manifest.cells.iter().copied().collect();
        Ok(Self {
            root,
            manifest,
            config: StreamingConfig::default(),
            catalog,
            loaded: HashMap::new(),
            memory: HashMap::new(),
        })
    }

    pub fn in_memory(
        cell_size: f32,
        origin: [f32; 3],
        cells: HashMap<PartitionCell, Vec<ActorRecord>>,
    ) -> Self {
        let catalog: HashSet<PartitionCell> = cells.keys().copied().collect();
        let mut manifest = WorldManifest::new("memory", cell_size, 0);
        manifest.origin = origin;
        manifest.cells = catalog.iter().copied().collect();
        Self {
            root: PathBuf::new(),
            manifest,
            config: StreamingConfig::default(),
            catalog,
            loaded: HashMap::new(),
            memory: cells,
        }
    }

    pub fn cell_size(&self) -> f32 {
        self.manifest.cell_size
    }

    pub fn cell_at(&self, world_pos: Vec3) -> PartitionCell {
        PartitionCell::from_world(
            self.manifest.cell_size,
            self.manifest.origin,
            world_pos.to_array(),
        )
    }

    pub fn is_loaded(&self, cell: PartitionCell) -> bool {
        self.loaded.contains_key(&cell)
    }

    pub fn loaded_cells(&self) -> impl Iterator<Item = PartitionCell> + '_ {
        self.loaded.keys().copied()
    }

    pub fn loaded_entity_count(&self) -> usize {
        self.loaded.values().map(|c| c.entities.len()).sum()
    }

    /// Editor: force-load a cell regardless of streaming budget.
    pub fn force_load(&mut self, world: &mut World, cell: PartitionCell) -> SceneResult<()> {
        if self.is_loaded(cell) {
            return Ok(());
        }
        self.catalog.insert(cell);
        self.load_cell(world, cell)
    }

    /// Editor: force-unload a cell.
    pub fn force_unload(&mut self, world: &mut World, cell: PartitionCell) {
        self.unload_cell(world, cell);
    }

    /// Disk/catalog cells known to this partition.
    pub fn catalog_cells(&self) -> impl Iterator<Item = PartitionCell> + '_ {
        self.catalog.iter().copied()
    }

    /// Load / unload cells around `viewer_pos`, respecting hysteresis and budget.
    pub fn tick(&mut self, world: &mut World, viewer_pos: Vec3) -> StreamingReport {
        let mut report = StreamingReport::default();
        report.written = self.writeback_dirty(world);

        let viewer = self.cell_at(viewer_pos);
        self.reload_changed(world, &mut report);

        let mut want = HashSet::new();
        for dy in -self.config.load_radius..=self.config.load_radius {
            for dx in -self.config.load_radius..=self.config.load_radius {
                let cell = PartitionCell {
                    x: viewer.x + dx,
                    y: viewer.y + dy,
                };
                if self.catalog.contains(&cell) {
                    want.insert(cell);
                }
            }
        }

        let unload_r = self.config.unload_radius.max(self.config.load_radius);
        let mut to_unload: Vec<_> = self
            .loaded
            .keys()
            .copied()
            .filter(|c| (c.x - viewer.x).abs().max((c.y - viewer.y).abs()) > unload_r)
            .collect();
        to_unload.sort_by_key(|c| (c.y, c.x));
        to_unload.truncate(self.config.budget.max_unloads);

        let mut to_load: Vec<_> = want
            .into_iter()
            .filter(|c| !self.loaded.contains_key(c))
            .collect();
        to_load.sort_by_key(|c| {
            let d = (c.x - viewer.x).abs() + (c.y - viewer.y).abs();
            (d, c.y, c.x)
        });
        to_load.truncate(self.config.budget.max_loads);

        for cell in to_unload {
            self.unload_cell(world, cell);
            report.unloaded.push(cell);
        }
        for cell in to_load {
            if self.load_cell(world, cell).is_ok() {
                report.loaded.push(cell);
            }
        }
        report
    }

    fn load_cell(&mut self, world: &mut World, cell: PartitionCell) -> SceneResult<()> {
        let actors = if let Some(mem) = self.memory.get(&cell) {
            mem.clone()
        } else {
            load_cell_actors(&self.root, cell)?
        };
        let mtime = cell_mtime(&self.root, cell);
        let mut entities = Vec::with_capacity(actors.len());
        for actor in actors {
            let e = spawn_actor(world, cell, &actor);
            entities.push(e);
        }
        self.loaded.insert(cell, LoadedCell { entities, mtime });
        Ok(())
    }

    fn unload_cell(&mut self, world: &mut World, cell: PartitionCell) {
        if let Some(loaded) = self.loaded.remove(&cell) {
            for e in loaded.entities {
                world.despawn(e);
            }
        }
    }

    fn reload_changed(&mut self, world: &mut World, report: &mut StreamingReport) {
        if self.root.as_os_str().is_empty() {
            return;
        }
        let cells: Vec<_> = self.loaded.keys().copied().collect();
        for cell in cells {
            let Some(disk) = cell_mtime(&self.root, cell) else {
                continue;
            };
            let stale = self
                .loaded
                .get(&cell)
                .and_then(|l| l.mtime)
                .is_none_or(|known| disk > known);
            if stale {
                self.unload_cell(world, cell);
                if self.load_cell(world, cell).is_ok() {
                    report.reloaded.push(cell);
                }
            }
        }
    }

    fn writeback_dirty(&self, world: &World) -> usize {
        if self.root.as_os_str().is_empty() {
            return 0;
        }
        let q = Query::<(&StreamedActor, &Transform), Changed<Transform>>::new();
        let mut n = 0;
        for (actor, xf) in q.iter(world) {
            let record = ActorRecord {
                id: actor.id.clone(),
                kind: actor.kind.clone(),
                position: xf.translation.to_array(),
                yaw: yaw_from_quat(xf.rotation),
                tags: Vec::new(),
            };
            if save_actor(&self.root, actor.cell, &record).is_ok() {
                n += 1;
            }
        }
        n
    }
}

fn spawn_actor(world: &mut World, cell: PartitionCell, actor: &ActorRecord) -> Entity {
    world.spawn_bundle((
        StreamedActor {
            cell,
            id: actor.id.clone(),
            kind: actor.kind.clone(),
        },
        Transform {
            translation: Vec3::from_array(actor.position),
            rotation: Quat::from_rotation_y(actor.yaw),
            scale: Vec3::ONE,
        },
    ))
}

fn yaw_from_quat(q: Quat) -> f32 {
    let f = q * Vec3::Z;
    f.x.atan2(f.z)
}

/// Pulls [`WorldPartition`] out of the ECS world, ticks it, puts it back.
///
/// ```ignore
/// world.insert_resource(WorldPartition::open("worlds/arena")?);
/// // each frame:
/// redox_scene::tick_streaming(world, camera_pos);
/// ```
pub fn tick_streaming(world: &mut World, viewer_pos: Vec3) -> Option<StreamingReport> {
    let mut partition = world.remove_resource::<WorldPartition>()?;
    let report = partition.tick(world, viewer_pos);
    world.insert_resource(partition);
    Some(report)
}
