//! `bake-hlod`: one `.hlod` proxy per world cell.
//!
//! Each actor reaches its geometry through the actor kind table
//! (`actor_kinds.ron`, see `redox_scene::kinds`). For every cell the actors
//! become mesh instances, the instances are merged into one proxy by
//! `redox_render::hlod::build_proxy`, and the proxy is written as `<x>_<y>.hlod`.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use redox_render::hlod::{HlodInput, HlodParams, build_proxy};
use redox_scene::kinds::{ActorKindTable, HlodInstance};
use redox_scene::world_bin::BinWorld;

/// Default triangle budget of one cell proxy.
pub const DEFAULT_HLOD_TRIANGLES: usize = 2000;
/// Name of the kind table looked up next to the world file.
pub const KINDS_FILE: &str = "actor_kinds.ron";
/// Largest proxy error, as a fraction of the cell diagonal.
const MAX_ERROR_FRACTION: f32 = 0.05;
/// Vertices closer than this (world units) are welded.
const WELD_EPSILON: f32 = 1e-4;

/// Options of `bake-hlod`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HlodOptions {
    /// Triangle budget of each proxy.
    pub triangles: usize,
    /// Kind table to use; defaults to `actor_kinds.ron` next to the world file.
    pub kinds: Option<PathBuf>,
}

/// Parses the options after `bake-hlod <world> <out>`: `--triangles N` and/or
/// `--kinds <file>`, in any order.
pub fn parse_options(flags: &[&str]) -> Result<HlodOptions, String> {
    let mut options = HlodOptions {
        triangles: DEFAULT_HLOD_TRIANGLES,
        kinds: None,
    };
    let mut rest = flags;
    while let [flag, value, tail @ ..] = rest {
        match *flag {
            "--triangles" => {
                options.triangles = value
                    .parse::<usize>()
                    .ok()
                    .filter(|&n| n > 0)
                    .ok_or_else(|| format!("bad --triangles value: {value}"))?;
            }
            "--kinds" => options.kinds = Some(PathBuf::from(*value)),
            _ => return Err(format!("unknown option: {}", flags.join(" "))),
        }
        rest = tail;
    }
    if let [flag] = rest {
        return Err(format!("option without value: {flag}"));
    }
    Ok(options)
}

/// Triangles of one mesh primitive, in the mesh's local space.
struct MeshPart {
    positions: Vec<[f32; 3]>,
    indices: Vec<u32>,
}

/// Reads every triangle primitive of a glTF/GLB file (node transforms are ignored;
/// the actor transform is applied instead).
fn load_mesh_parts(path: &Path) -> Result<Vec<MeshPart>, String> {
    let (document, buffers, _) =
        gltf::import(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut parts = Vec::new();
    for mesh in document.meshes() {
        for primitive in mesh.primitives() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(|d| &d.0[..]));
            let positions: Vec<[f32; 3]> = reader
                .read_positions()
                .ok_or_else(|| format!("{}: primitive without positions", path.display()))?
                .collect();
            let indices = match reader.read_indices() {
                Some(indices) => indices.into_u32().collect(),
                None => (0..positions.len() as u32).collect(),
            };
            parts.push(MeshPart { positions, indices });
        }
    }
    if parts.is_empty() {
        return Err(format!("{}: no triangle primitives", path.display()));
    }
    Ok(parts)
}

/// Loads the meshes of `instances` that are not in `cache` yet.
fn ensure_meshes(
    cache: &mut HashMap<String, Vec<MeshPart>>,
    instances: &[HlodInstance],
    mesh_dir: &Path,
) -> Result<(), String> {
    for instance in instances {
        if !cache.contains_key(&instance.mesh) {
            let parts = load_mesh_parts(&mesh_dir.join(&instance.mesh))?;
            cache.insert(instance.mesh.clone(), parts);
        }
    }
    Ok(())
}

/// One proxy input per mesh primitive of each instance.
fn cell_inputs<'a>(
    cache: &'a HashMap<String, Vec<MeshPart>>,
    instances: &[HlodInstance],
) -> Result<Vec<HlodInput<'a>>, String> {
    let mut inputs = Vec::new();
    for instance in instances {
        let parts = cache
            .get(&instance.mesh)
            .ok_or_else(|| format!("mesh not loaded: {}", instance.mesh))?;
        for part in parts {
            inputs.push(HlodInput {
                positions: &part.positions,
                indices: &part.indices,
                transform: instance.transform,
                color: instance.color,
            });
        }
    }
    Ok(inputs)
}

/// Bakes one `.hlod` proxy per non-empty cell of the binary world `world` into
/// `out`. Returns the number of files written.
///
/// # Errors
/// A readable message when the world or kind table cannot be read, a mesh cannot
/// be loaded, a proxy cannot be built, or a file cannot be written.
pub fn bake_hlod(world: &Path, out: &Path, options: &HlodOptions) -> Result<usize, String> {
    let mut bin = BinWorld::open(world).map_err(|e| format!("{}: {e}", world.display()))?;
    let kinds_path = options.kinds.clone().unwrap_or_else(|| {
        world
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(KINDS_FILE)
    });
    let kinds_text = fs::read_to_string(&kinds_path)
        .map_err(|e| format!("cannot read {}: {e}", kinds_path.display()))?;
    let table = ActorKindTable::from_ron(&kinds_text)
        .map_err(|e| format!("{}: {e}", kinds_path.display()))?;
    let mesh_dir = kinds_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();

    let cell_size = bin.manifest().cell_size;
    let params = HlodParams {
        target_triangles: options.triangles,
        max_error: MAX_ERROR_FRACTION * cell_size * std::f32::consts::SQRT_2,
        weld_epsilon: WELD_EPSILON,
    };
    fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;

    let cells: Vec<_> = bin.cells().collect();
    let mut cache = HashMap::new();
    let mut warned = BTreeSet::new();
    let mut written = 0;
    for cell in cells {
        let actors = bin
            .load_cell(cell)
            .map_err(|e| format!("cell {}_{}: {e}", cell.x, cell.y))?;
        let (instances, unknown) = table.to_instances(&actors);
        for kind in unknown.difference(&warned) {
            eprintln!(
                "warning: actor kind {kind:?} has no entry in {}; its actors are skipped",
                kinds_path.display()
            );
        }
        warned.extend(unknown);
        if instances.is_empty() {
            continue;
        }
        ensure_meshes(&mut cache, &instances, &mesh_dir)?;
        let inputs = cell_inputs(&cache, &instances)?;
        let proxy = build_proxy(&inputs, &params)
            .map_err(|e| format!("cell {}_{}: {e}", cell.x, cell.y))?;
        let file = out.join(format!("{}_{}.hlod", cell.x, cell.y));
        fs::write(&file, proxy.to_bytes((cell.x, cell.y)))
            .map_err(|e| format!("{}: {e}", file.display()))?;
        written += 1;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_options_are_used_without_flags() {
        let options = parse_options(&[]).unwrap();
        assert_eq!(options.triangles, 2000);
        assert_eq!(options.kinds, None);
    }

    #[test]
    fn a_triangle_count_and_a_kind_table_are_parsed_in_any_order() {
        let options = parse_options(&["--triangles", "500", "--kinds", "k.ron"]).unwrap();
        assert_eq!(options.triangles, 500);
        assert_eq!(options.kinds, Some(PathBuf::from("k.ron")));
        let swapped = parse_options(&["--kinds", "k.ron", "--triangles", "500"]).unwrap();
        assert_eq!(swapped, options);
    }

    #[test]
    fn bad_flags_are_errors() {
        assert!(parse_options(&["--triangles"]).is_err());
        assert!(parse_options(&["--triangles", "many"]).is_err());
        assert!(parse_options(&["--triangles", "0"]).is_err());
        assert!(parse_options(&["--wat", "1"]).is_err());
        assert!(parse_options(&["--kinds"]).is_err());
    }

    #[test]
    fn cells_without_known_kinds_write_no_files() {
        use redox_scene::world_bin::write_world_bin;
        use redox_scene::{ActorRecord, PartitionCell, WorldManifest};

        let dir = std::env::temp_dir().join(format!("redox_hlod_bake_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let world = dir.join("world.bin");
        let actor = ActorRecord {
            id: "a".into(),
            kind: "unknown".into(),
            position: [1.0, 0.0, 1.0],
            yaw: 0.0,
            tags: Vec::new(),
        };
        let cells = [
            (PartitionCell { x: 0, y: 0 }, vec![actor.clone()]),
            (PartitionCell { x: 1, y: 0 }, vec![actor]),
        ];
        write_world_bin(&world, WorldManifest::new("test", 50.0, 1), &cells).unwrap();
        fs::write(dir.join(KINDS_FILE), "(kinds: {})").unwrap();

        let written = bake_hlod(&world, &dir.join("out"), &parse_options(&[]).unwrap());
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(written, Ok(0));
    }
}
