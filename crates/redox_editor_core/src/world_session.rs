use std::path::{Path, PathBuf};

use redox_ecs::World;
use redox_math::Vec3;
use redox_scene::ofpa::{self, ActorRecord, WorldManifest};
use redox_scene::{PartitionCell, SceneResult, WorldPartition};

/// The world open in the editor.
pub struct WorldSession {
    /// Streams cells in and out as the viewer moves.
    pub partition: WorldPartition,
    /// Header of the world file.
    pub manifest: WorldManifest,
    /// Directory the world is stored in.
    pub root: PathBuf,
}

impl WorldSession {
    /// Opens the world stored under `root`.
    pub fn open(root: impl AsRef<Path>) -> SceneResult<Self> {
        let root = root.as_ref().to_path_buf();
        let partition = WorldPartition::open(&root)?;
        let manifest =
            ofpa::load_manifest(&root).unwrap_or_else(|_| WorldManifest::new("untitled", 16.0, 42));
        Ok(Self {
            partition,
            manifest,
            root,
        })
    }

    /// Alias for [`Self::open`] — path-oriented name for menus / status flags.
    pub fn open_path(path: impl AsRef<Path>) -> SceneResult<Self> {
        Self::open(path)
    }

    /// List world directories under `worlds/` (cwd-relative), preferring those with
    /// `world.ron` or a `cells/` folder. Falls back to every subdirectory.
    pub fn suggest_world_roots() -> Vec<PathBuf> {
        Self::suggest_world_roots_in("worlds")
    }

    /// Scan `base` for candidate world roots.
    pub fn suggest_world_roots_in(base: impl AsRef<Path>) -> Vec<PathBuf> {
        let base = base.as_ref();
        let mut preferred = Vec::new();
        let mut other = Vec::new();
        let Ok(entries) = std::fs::read_dir(base) else {
            return preferred;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let has_manifest = path.join("world.ron").is_file();
            let has_cells = path.join(ofpa::CELLS_DIR).is_dir();
            if has_manifest || has_cells {
                preferred.push(path);
            } else {
                other.push(path);
            }
        }
        preferred.sort();
        other.sort();
        if preferred.is_empty() {
            other
        } else {
            preferred
        }
    }

    /// Creates the world directory with a default manifest (cell size 16, seed 42) and opens it;
    /// the kind and size arguments are not used yet.
    pub fn create_new(
        root: impl AsRef<Path>,
        _kind: &str,
        _width: u32,
        _height: u32,
    ) -> SceneResult<Self> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(&root)?;
        let manifest = WorldManifest::new(
            root.file_name().and_then(|s| s.to_str()).unwrap_or("world"),
            16.0,
            42,
        );
        ofpa::save_manifest(&root, &manifest)?;
        let partition = WorldPartition::open(&root)?;
        Ok(Self {
            partition,
            manifest,
            root,
        })
    }

    /// Streams cells around `viewer_pos` into and out of `world`.
    pub fn tick(&mut self, world: &mut World, viewer_pos: Vec3) {
        let _ = self.partition.tick(world, viewer_pos);
    }

    /// Loads a cell regardless of the streaming distance.
    pub fn force_load(&mut self, world: &mut World, cell: PartitionCell) -> SceneResult<()> {
        self.partition.force_load(world, cell)
    }

    /// Unloads a cell regardless of the streaming distance.
    pub fn force_unload(&mut self, world: &mut World, cell: PartitionCell) {
        self.partition.force_unload(world, cell);
    }

    /// Whether the cell is currently loaded.
    pub fn is_loaded(&self, cell: PartitionCell) -> bool {
        self.partition.is_loaded(cell)
    }

    /// Scan `cells/` on disk (plus catalog) for outliner.
    pub fn list_cells(&self) -> Vec<PartitionCell> {
        let mut set: std::collections::BTreeSet<(i32, i32)> = std::collections::BTreeSet::new();
        for c in self.partition.catalog_cells() {
            set.insert((c.x, c.y));
        }
        let cells_root = self.root.join(ofpa::CELLS_DIR);
        if let Ok(entries) = std::fs::read_dir(&cells_root) {
            for e in entries.flatten() {
                if let Some(name) = e.file_name().to_str()
                    && let Some(cell) = ofpa::parse_cell_dir(name)
                {
                    set.insert((cell.x, cell.y));
                }
            }
        }
        set.into_iter()
            .map(|(x, y)| PartitionCell { x, y })
            .collect()
    }

    /// Actors stored for a cell on disk.
    pub fn list_cell_actors(&self, cell: PartitionCell) -> Vec<ActorRecord> {
        ofpa::load_cell_actors(&self.root, cell).unwrap_or_default()
    }

    /// Marker for the cell list: green when loaded, yellow when within the load radius of the
    /// viewer's cell, black otherwise.
    pub fn cell_status_emoji(&self, cell: PartitionCell, viewer: PartitionCell) -> &'static str {
        if self.is_loaded(cell) {
            "🟩"
        } else {
            let d = (cell.x - viewer.x).abs().max((cell.y - viewer.y).abs());
            if d <= self.partition.config.load_radius + 1 {
                "🟨"
            } else {
                "⬛"
            }
        }
    }

    /// Grid line segments around the world origin; the flag is set for the axis lines.
    pub fn grid_lines(&self) -> Vec<(Vec3, Vec3, bool)> {
        let cell_size = self.manifest.cell_size;
        let half = 3.0 * cell_size;
        let mut lines = Vec::new();
        for i in -3..=3 {
            let pos = i as f32 * cell_size;
            lines.push((
                Vec3::new(-half, 0.0, pos),
                Vec3::new(half, 0.0, pos),
                i == 0,
            ));
            lines.push((
                Vec3::new(pos, 0.0, -half),
                Vec3::new(pos, 0.0, half),
                i == 0,
            ));
        }
        lines
    }

    /// Paint brush: spawn props into OFPA cell + ECS.
    pub fn paint_actors(
        &mut self,
        world: &mut World,
        positions: &[(Vec3, f32)],
        kind: &str,
    ) -> usize {
        let mut n = 0;
        let mut touched = std::collections::HashSet::new();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        for (i, (pos, yaw)) in positions.iter().enumerate() {
            let cell = self.partition.cell_at(*pos);
            let id = format!("brush_{}_{}_{}_{}", cell.x, cell.y, kind, stamp + i as u128);
            let record = ActorRecord {
                id,
                kind: kind.to_string(),
                position: pos.to_array(),
                yaw: *yaw,
                tags: vec!["brush".into()],
            };
            if ofpa::save_actor(&self.root, cell, &record).is_ok() {
                touched.insert((cell.x, cell.y));
                n += 1;
            }
        }
        for (x, y) in touched {
            let cell = PartitionCell { x, y };
            if self.is_loaded(cell) {
                self.force_unload(world, cell);
            }
            let _ = self.force_load(world, cell);
        }
        n
    }
}
