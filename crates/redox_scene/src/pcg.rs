//! Procedural cell/dungeon generation used by the `pcg` CLI and tests.

use crate::ofpa::{write_world, ActorRecord, WorldManifest};
use crate::{PartitionCell, SceneResult};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcgKind {
    Arena,
    Dungeon,
}

impl PcgKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "arena" => Some(Self::Arena),
            "dungeon" => Some(Self::Dungeon),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PcgParams {
    pub name: String,
    pub kind: PcgKind,
    pub width: i32,
    pub height: i32,
    pub cell_size: f32,
    pub seed: u64,
}

impl Default for PcgParams {
    fn default() -> Self {
        Self {
            name: "world".into(),
            kind: PcgKind::Arena,
            width: 8,
            height: 8,
            cell_size: 64.0,
            seed: 1,
        }
    }
}

pub struct GeneratedWorld {
    pub manifest: WorldManifest,
    pub cells: Vec<(PartitionCell, Vec<ActorRecord>)>,
}

impl GeneratedWorld {
    pub fn write(&self, root: impl AsRef<Path>) -> SceneResult<()> {
        write_world(root, self.manifest.clone(), &self.cells)
    }
}

/// Mixes `seed` with cell coords into a deterministic 64-bit value.
pub fn cell_seed(world_seed: u64, cell: PartitionCell) -> u64 {
    splitmix64(
        world_seed
            ^ (cell.x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (cell.y as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9),
    )
}

pub fn generate(params: &PcgParams) -> GeneratedWorld {
    let occupied = match params.kind {
        PcgKind::Arena => arena_cells(params.width, params.height),
        PcgKind::Dungeon => dungeon_cells(params.width, params.height, params.seed),
    };

    let mut cells = Vec::new();
    for cell in &occupied {
        cells.push((*cell, generate_cell(params, *cell, &occupied)));
    }
    cells.sort_by_key(|(c, _)| (c.y, c.x));

    let mut manifest = WorldManifest::new(&params.name, params.cell_size, params.seed);
    manifest.cells = cells.iter().map(|(c, _)| *c).collect();

    GeneratedWorld { manifest, cells }
}

fn arena_cells(w: i32, h: i32) -> HashSet<PartitionCell> {
    let mut set = HashSet::new();
    for y in 0..h {
        for x in 0..w {
            set.insert(PartitionCell { x, y });
        }
    }
    set
}

fn dungeon_cells(w: i32, h: i32, seed: u64) -> HashSet<PartitionCell> {
    let mut rng = seed;
    let mut set = HashSet::new();
    let mut x = w / 2;
    let mut y = h / 2;
    let steps = (w * h * 3).max(16);
    for _ in 0..steps {
        set.insert(PartitionCell { x, y });
        rng = splitmix64(rng);
        match rng % 4 {
            0 => x = (x + 1).clamp(0, w - 1),
            1 => x = (x - 1).clamp(0, w - 1),
            2 => y = (y + 1).clamp(0, h - 1),
            _ => y = (y - 1).clamp(0, h - 1),
        }
    }
    set
}

fn generate_cell(
    params: &PcgParams,
    cell: PartitionCell,
    occupied: &HashSet<PartitionCell>,
) -> Vec<ActorRecord> {
    let size = params.cell_size;
    let origin_x = cell.x as f32 * size + size * 0.5;
    let origin_z = cell.y as f32 * size + size * 0.5;
    let mut actors = Vec::new();
    let mut rng = cell_seed(params.seed, cell);

    actors.push(ActorRecord {
        id: format!("floor_{}_{}", cell.x, cell.y),
        kind: "floor".into(),
        position: [origin_x, 0.0, origin_z],
        yaw: 0.0,
        tags: vec!["pcg".into()],
    });

    let border = matches!(params.kind, PcgKind::Arena)
        && (cell.x == 0 || cell.y == 0 || cell.x == params.width - 1 || cell.y == params.height - 1);
    let dungeon_wall = matches!(params.kind, PcgKind::Dungeon)
        && [
            PartitionCell { x: cell.x + 1, y: cell.y },
            PartitionCell { x: cell.x - 1, y: cell.y },
            PartitionCell { x: cell.x, y: cell.y + 1 },
            PartitionCell { x: cell.x, y: cell.y - 1 },
        ]
        .iter()
        .any(|n| !occupied.contains(n));

    if border || dungeon_wall {
        actors.push(ActorRecord {
            id: format!("wall_{}_{}", cell.x, cell.y),
            kind: "wall".into(),
            position: [origin_x, 1.0, origin_z],
            yaw: 0.0,
            tags: vec!["pcg".into()],
        });
    }

    rng = splitmix64(rng);
    let scatter_n = (rng % 4) as i32;
    for i in 0..scatter_n {
        rng = splitmix64(rng);
        // Mask to 32 bits before f32 cast — `(u64 >> 8) as f32` keeps ~56-bit
        // magnitude and flings props to 1e7–1e8 m (breaks HDR / precision).
        let ox = ((((rng >> 8) as u32) as f32 / u32::MAX as f32) - 0.5) * size * 0.6;
        rng = splitmix64(rng);
        let oz = ((((rng >> 40) as u32) as f32 / u32::MAX as f32) - 0.5) * size * 0.6;
        rng = splitmix64(rng);
        actors.push(ActorRecord {
            id: format!("prop_{}_{}_{}", cell.x, cell.y, i),
            kind: "prop".into(),
            position: [origin_x + ox, 0.5, origin_z + oz],
            yaw: (rng as f32 / u64::MAX as f32) * std::f32::consts::TAU,
            tags: vec!["pcg".into(), "scatter".into()],
        });
    }

    if cell.x == params.width / 2 && cell.y == params.height / 2 {
        actors.push(ActorRecord {
            id: "spawn".into(),
            kind: "spawn".into(),
            position: [origin_x, 0.0, origin_z],
            yaw: 0.0,
            tags: vec!["player_start".into()],
        });
    }

    actors
}

fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Convenience: generate and write in one call.
pub fn generate_to(root: impl AsRef<Path>, params: &PcgParams) -> SceneResult<GeneratedWorld> {
    let world = generate(params);
    world.write(root)?;
    Ok(world)
}

/// Groups records by cell (for callers that already have a flat actor list).
pub fn index_by_cell(
    cell_size: f32,
    origin: [f32; 3],
    actors: impl IntoIterator<Item = ActorRecord>,
) -> HashMap<PartitionCell, Vec<ActorRecord>> {
    let mut map: HashMap<PartitionCell, Vec<ActorRecord>> = HashMap::new();
    for actor in actors {
        let cell = PartitionCell::from_world(cell_size, origin, actor.position);
        map.entry(cell).or_default().push(actor);
    }
    map
}
