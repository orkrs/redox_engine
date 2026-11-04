//! Procedural cell/dungeon generation used by the `pcg` CLI and tests.

use crate::ofpa::{ActorRecord, WorldManifest, write_world};
use crate::{PartitionCell, SceneResult};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Kind of generated world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcgKind {
    /// A filled rectangle of cells.
    Arena,
    /// A seed-dependent layout of occupied cells.
    Dungeon,
}

impl PcgKind {
    /// Parses `"arena"` or `"dungeon"`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "arena" => Some(Self::Arena),
            "dungeon" => Some(Self::Dungeon),
            _ => None,
        }
    }
}

/// Parameters of world generation.
#[derive(Clone, Debug)]
pub struct PcgParams {
    /// Name written to the manifest.
    pub name: String,
    /// Kind of world.
    pub kind: PcgKind,
    /// Grid width in cells.
    pub width: i32,
    /// Grid height in cells.
    pub height: i32,
    /// Cell edge length in world units.
    pub cell_size: f32,
    /// Seed; the same seed gives the same world.
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

/// A generated world, not yet written to disk.
pub struct GeneratedWorld {
    /// Manifest describing the cells.
    pub manifest: WorldManifest,
    /// Actors per cell.
    pub cells: Vec<(PartitionCell, Vec<ActorRecord>)>,
}

impl GeneratedWorld {
    /// Writes the manifest and all cells under `root`.
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

/// Generates a world deterministically from the parameters.
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
        && (cell.x == 0
            || cell.y == 0
            || cell.x == params.width - 1
            || cell.y == params.height - 1);
    let dungeon_wall = matches!(params.kind, PcgKind::Dungeon)
        && [
            PartitionCell {
                x: cell.x + 1,
                y: cell.y,
            },
            PartitionCell {
                x: cell.x - 1,
                y: cell.y,
            },
            PartitionCell {
                x: cell.x,
                y: cell.y + 1,
            },
            PartitionCell {
                x: cell.x,
                y: cell.y - 1,
            },
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

/// Rule of a foliage layer scattered over the cells (FOL-03 step 5).
#[derive(Clone, Debug)]
pub struct FoliageRule {
    /// Model the layer draws.
    pub model: String,
    /// Expected placements per square metre, before the mask and the filters.
    pub density_per_m2: f32,
    /// Smallest scale of a placement.
    pub min_scale: f32,
    /// Largest scale of a placement.
    pub max_scale: f32,
    /// Steepest accepted slope, in degrees.
    pub max_slope: f32,
    /// Accepted height range in metres, inclusive on both ends.
    pub height_range: (f32, f32),
    /// Optional mask over world `(x, z)`; `false` leaves the spot empty.
    pub mask: Option<fn(f32, f32) -> bool>,
}

/// One placement made by [`scatter_layer`]. Converts field by field to the render instance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScatteredInstance {
    /// World position; `y` is the height at `(x, z)`.
    pub pos: [f32; 3],
    /// Rotation around Y, in radians.
    pub yaw: f32,
    /// Uniform scale.
    pub scale: f32,
    /// Per-instance random seed.
    pub seed: u32,
}

/// Distance of the finite differences that give the slope.
const SLOPE_STEP: f32 = 0.5;

/// Scatters the instances of `rule` in one cell. Deterministic in `(seed, rule.model, cell)`:
/// every candidate draws the same five random values whether it is accepted or not, so one
/// rejected spot never shifts the others. Instances stay inside the cell's square of side
/// `cell_size`, so neighbouring cells never share a placement.
pub fn scatter_layer(
    rule: &FoliageRule,
    cell: PartitionCell,
    cell_size: f32,
    seed: u64,
    height_fn: impl Fn(f32, f32) -> f32,
) -> Vec<ScatteredInstance> {
    let expected = (rule.density_per_m2 * cell_size * cell_size).max(0.0);
    let mut rng = cell_seed(seed ^ name_hash(&rule.model), cell);
    rng = splitmix64(rng);
    let whole = expected.floor();
    let count = whole as u32 + u32::from(unit(rng) < expected - whole);

    let (x0, z0) = (cell.x as f32 * cell_size, cell.y as f32 * cell_size);
    let mut out = Vec::new();
    for _ in 0..count {
        let x = x0 + unit(next(&mut rng)) * cell_size;
        let z = z0 + unit(next(&mut rng)) * cell_size;
        let yaw = unit(next(&mut rng)) * std::f32::consts::TAU;
        let scale = rule.min_scale + (rule.max_scale - rule.min_scale) * unit(next(&mut rng));
        let instance_seed = (next(&mut rng) >> 32) as u32;
        if !rule.mask.is_none_or(|mask| mask(x, z)) {
            continue;
        }
        let height = height_fn(x, z);
        if height < rule.height_range.0 || height > rule.height_range.1 {
            continue;
        }
        if slope_degrees(&height_fn, x, z) > rule.max_slope {
            continue;
        }
        out.push(ScatteredInstance {
            pos: [x, height, z],
            yaw,
            scale,
            seed: instance_seed,
        });
    }
    out
}

/// Advances `state` and returns the new value (splitmix stream).
fn next(state: &mut u64) -> u64 {
    *state = splitmix64(*state);
    *state
}

/// Uniform value in `[0, 1)` from the top 24 bits.
fn unit(bits: u64) -> f32 {
    (bits >> 40) as f32 / (1u64 << 24) as f32
}

/// Slope in degrees from central differences of the height.
fn slope_degrees(height_fn: &impl Fn(f32, f32) -> f32, x: f32, z: f32) -> f32 {
    let gx = (height_fn(x + SLOPE_STEP, z) - height_fn(x - SLOPE_STEP, z)) / (2.0 * SLOPE_STEP);
    let gz = (height_fn(x, z + SLOPE_STEP) - height_fn(x, z - SLOPE_STEP)) / (2.0 * SLOPE_STEP);
    (1.0 / (gx * gx + gz * gz + 1.0).sqrt()).acos().to_degrees()
}

/// FNV-1a of the model name, so two rules with the same seed scatter differently.
fn name_hash(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    const CELL: f32 = 64.0;

    fn rule(max_slope: f32, height_range: (f32, f32)) -> FoliageRule {
        FoliageRule {
            model: "fern".into(),
            density_per_m2: 0.5,
            min_scale: 0.8,
            max_scale: 1.2,
            max_slope,
            height_range,
            mask: None,
        }
    }

    /// Flat ground at height 0.
    fn flat(_: f32, _: f32) -> f32 {
        0.0
    }

    /// A ramp rising 0.5 m per metre along X (slope about 26.6 degrees).
    fn ramp(x: f32, _: f32) -> f32 {
        0.5 * x
    }

    #[test]
    fn scatter_is_deterministic() {
        let r = rule(45.0, (-1.0, 1.0));
        let cell = PartitionCell { x: 3, y: -2 };
        let a = scatter_layer(&r, cell, CELL, 42, flat);
        let b = scatter_layer(&r, cell, CELL, 42, flat);
        assert!(!a.is_empty());
        assert_eq!(a, b);
        assert_ne!(a, scatter_layer(&r, cell, CELL, 43, flat));
    }

    #[test]
    fn scatter_respects_slope_and_height() {
        let cell = PartitionCell { x: 0, y: 0 };
        let steep = scatter_layer(&rule(20.0, (-1.0, 100.0)), cell, CELL, 7, ramp);
        assert!(steep.is_empty(), "a 26.6 degree slope passed a 20 degree limit");

        let gentle = scatter_layer(&rule(30.0, (0.0, 10.0)), cell, CELL, 7, ramp);
        assert!(!gentle.is_empty());
        for inst in &gentle {
            assert!((0.0..=10.0).contains(&inst.pos[1]), "height {}", inst.pos[1]);
            assert_eq!(inst.pos[1], ramp(inst.pos[0], inst.pos[2]));
        }
    }

    #[test]
    fn adjacent_cells_do_not_overlap_instances() {
        let r = rule(90.0, (-1.0, 1.0));
        let left = scatter_layer(&r, PartitionCell { x: 0, y: 0 }, CELL, 9, flat);
        let right = scatter_layer(&r, PartitionCell { x: 1, y: 0 }, CELL, 9, flat);
        assert!(!left.is_empty() && !right.is_empty());
        assert!(left.iter().all(|i| (0.0..CELL).contains(&i.pos[0])));
        assert!(right.iter().all(|i| (CELL..2.0 * CELL).contains(&i.pos[0])));
        assert!(left.iter().all(|a| right.iter().all(|b| a.pos != b.pos)));
    }
}
