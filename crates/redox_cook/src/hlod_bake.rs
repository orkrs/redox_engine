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

/// Where the colour of a mesh primitive comes from.
#[derive(Debug, Clone, PartialEq)]
enum PartColour {
    /// glTF `COLOR_0` per vertex, multiplied by the material base colour.
    PerVertex(Vec<[f32; 3]>),
    /// The material base colour (the primitive has no vertex colours).
    Flat([f32; 3]),
    /// Neither: the colour of the actor kind table.
    Table,
}

/// Picks the colour source of a primitive from its vertex colours and its material.
fn choose_colour(vertex_colors: Option<Vec<[f32; 3]>>, base: Option<[f32; 3]>) -> PartColour {
    match (vertex_colors, base) {
        (Some(colors), base) => {
            let [r, g, b] = base.unwrap_or([1.0; 3]);
            PartColour::PerVertex(
                colors
                    .into_iter()
                    .map(|[cr, cg, cb]| [cr * r, cg * g, cb * b])
                    .collect(),
            )
        }
        (None, Some(base)) => PartColour::Flat(base),
        (None, None) => PartColour::Table,
    }
}

/// Triangles of one mesh primitive, in the mesh's local space.
struct MeshPart {
    positions: Vec<[f32; 3]>,
    indices: Vec<u32>,
    colour: PartColour,
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
            let vertex_colors = reader
                .read_colors(0)
                .map(|colors| colors.into_rgb_f32().collect::<Vec<_>>());
            // ASSUMPTION: a textured material contributes no flat colour; its primitives
            // fall back to the kind table colour (textures are ignored by the proxy).
            let material = primitive.material();
            let pbr = material.pbr_metallic_roughness();
            let base =
                (material.index().is_some() && pbr.base_color_texture().is_none()).then(|| {
                    let [r, g, b, _] = pbr.base_color_factor();
                    [r, g, b]
                });
            let colour = choose_colour(vertex_colors, base);
            parts.push(MeshPart {
                positions,
                indices,
                colour,
            });
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
            let (color, vertex_colors) = match &part.colour {
                PartColour::PerVertex(colors) => (instance.color, Some(colors.as_slice())),
                PartColour::Flat(flat) => (*flat, None),
                PartColour::Table => (instance.color, None),
            };
            inputs.push(HlodInput {
                positions: &part.positions,
                indices: &part.indices,
                transform: instance.transform,
                color,
                vertex_colors,
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

/// Options of `bake-sdf`.
#[derive(Debug, Clone, PartialEq)]
pub struct SdfOptions {
    /// Kind table; default `actor_kinds.ron` next to the world file.
    pub kinds: Option<PathBuf>,
    /// Largest brick edge, metres (a bigger mesh gets one brick of this size).
    pub max_world_size: f32,
}

/// Parses the flags of `bake-sdf`: `--kinds <file>`, `--max-world-size <m>`.
///
/// # Errors
/// A message for an unknown flag or a bad number.
pub fn parse_sdf_options(flags: &[&str]) -> Result<SdfOptions, String> {
    let mut options = SdfOptions {
        kinds: None,
        max_world_size: 8.0,
    };
    let mut it = flags.iter();
    while let Some(flag) = it.next() {
        match *flag {
            "--kinds" => {
                options.kinds = Some((*it.next().ok_or("--kinds needs a file")?).into());
            }
            "--max-world-size" => {
                let value = it.next().ok_or("--max-world-size needs a number")?;
                options.max_world_size = value
                    .parse::<f32>()
                    .ok()
                    .filter(|v| *v > 0.0)
                    .ok_or_else(|| format!("bad --max-world-size: {value}"))?;
            }
            other => return Err(format!("unknown flag: {other}")),
        }
    }
    Ok(options)
}

/// One brick for a mesh placed by `transform`: the mesh is moved to world space and baked
/// over a cube around its world AABB (edge = longest AABB edge plus a margin, at most
/// `max_world_size`). Rotated meshes are therefore baked in world axes.
pub fn bake_world_brick(
    positions: &[[f32; 3]],
    indices: &[u32],
    transform: redox_math::Mat4,
    max_world_size: f32,
) -> Option<redox_render::sdf::atlas::SdfBrick> {
    use redox_render::mesh::{Mesh, Vertex};
    use redox_render::sdf::atlas::SdfBrick;
    use redox_render::sdf::bake::bake_mesh;
    if positions.is_empty() || indices.len() < 3 {
        return None;
    }
    let vertices: Vec<Vertex> = positions
        .iter()
        .map(|p| Vertex {
            position: transform
                .transform_point3(redox_math::Vec3::from_array(*p))
                .to_array(),
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
            tangent: [1.0, 0.0, 0.0],
        })
        .collect();
    let mesh = Mesh::new(vertices, indices.to_vec());
    let size = (mesh.aabb.max - mesh.aabb.min).max_element();
    let edge = (size * 1.25).max(0.5).min(max_world_size);
    let world_size = redox_math::Vec3::splat(edge);
    let field = bake_mesh(&mesh, world_size);
    Some(SdfBrick {
        field,
        origin_ws: (mesh.aabb.min + mesh.aabb.max) * 0.5,
        world_size,
    })
}

/// Bakes one `<x>_<y>.sdf` per non-empty cell of `world` into `out`. Returns the file count.
/// Bricks are written in actor order, so two runs give equal bytes.
///
/// # Errors
/// A readable message when the world, the kind table or a mesh cannot be read or a file written.
pub fn bake_sdf(world: &Path, out: &Path, options: &SdfOptions) -> Result<usize, String> {
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
    fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let cells: Vec<_> = bin.cells().collect();
    let mut cache = HashMap::new();
    let mut written = 0;
    for cell in cells {
        let actors = bin
            .load_cell(cell)
            .map_err(|e| format!("cell {}_{}: {e}", cell.x, cell.y))?;
        let (instances, _unknown) = table.to_instances(&actors);
        if instances.is_empty() {
            continue;
        }
        ensure_meshes(&mut cache, &instances, &mesh_dir)?;
        let mut bricks = Vec::new();
        for instance in &instances {
            for part in cache.get(&instance.mesh).into_iter().flatten() {
                bricks.extend(bake_world_brick(
                    &part.positions,
                    &part.indices,
                    instance.transform,
                    options.max_world_size,
                ));
            }
        }
        if bricks.is_empty() {
            continue;
        }
        let file = out.join(format!("{}_{}.sdf", cell.x, cell.y));
        fs::write(&file, redox_render::sdf::format::write_bricks(&bricks))
            .map_err(|e| format!("{}: {e}", file.display()))?;
        written += 1;
    }
    Ok(written)
}

#[cfg(test)]
mod sdf_tests {
    use super::*;

    const CUBE: [[f32; 3]; 8] = [
        [-0.5, -0.5, -0.5],
        [0.5, -0.5, -0.5],
        [0.5, 0.5, -0.5],
        [-0.5, 0.5, -0.5],
        [-0.5, -0.5, 0.5],
        [0.5, -0.5, 0.5],
        [0.5, 0.5, 0.5],
        [-0.5, 0.5, 0.5],
    ];
    const CUBE_INDICES: [u32; 36] = [
        0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7, 3,
        1, 2, 6, 1, 6, 5,
    ];

    #[test]
    fn a_cube_gives_a_brick_centred_on_it() {
        let at = redox_math::Mat4::from_translation(redox_math::Vec3::new(10.0, 0.0, 5.0));
        let brick = bake_world_brick(&CUBE, &CUBE_INDICES, at, 8.0).unwrap();
        assert!((brick.origin_ws - redox_math::Vec3::new(10.0, 0.0, 5.0)).length() < 1e-4);
        // The centre of the cube is inside (the field is relative to the brick centre).
        assert!(brick.field.sample(redox_math::Vec3::ZERO) < 0.0);
    }

    #[test]
    fn a_big_mesh_gets_a_brick_no_larger_than_the_limit() {
        let big = redox_math::Mat4::from_scale(redox_math::Vec3::splat(40.0));
        let brick = bake_world_brick(&CUBE, &CUBE_INDICES, big, 8.0).unwrap();
        assert!(brick.world_size.max_element() <= 8.0 + 1e-4);
    }

    #[test]
    fn the_output_is_deterministic() {
        let a = bake_world_brick(&CUBE, &CUBE_INDICES, redox_math::Mat4::IDENTITY, 8.0).unwrap();
        let b = bake_world_brick(&CUBE, &CUBE_INDICES, redox_math::Mat4::IDENTITY, 8.0).unwrap();
        assert_eq!(
            redox_render::sdf::format::write_bricks(&[a]),
            redox_render::sdf::format::write_bricks(&[b])
        );
    }

    #[test]
    fn a_rotated_mesh_is_baked_in_world_axes() {
        // A cube turned 45 degrees about Y reaches 0.707 m along X.
        let turned = redox_math::Mat4::from_rotation_y(std::f32::consts::FRAC_PI_4);
        let brick = bake_world_brick(&CUBE, &CUBE_INDICES, turned, 8.0).unwrap();
        assert!(brick.field.sample(redox_math::Vec3::new(0.6, 0.0, 0.0)) < 0.0);
        assert!(brick.field.sample(redox_math::Vec3::new(0.9, 0.0, 0.0)) > 0.0);
    }

    #[test]
    fn the_flags_are_parsed() {
        let o = parse_sdf_options(&["--max-world-size", "4"]).unwrap();
        assert_eq!(o.max_world_size, 4.0);
        assert!(parse_sdf_options(&["--max-world-size", "0"]).is_err());
        assert!(parse_sdf_options(&["--bogus"]).is_err());
    }
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
    fn vertex_colours_are_multiplied_by_the_material_colour() {
        let colors = vec![[1.0, 1.0, 1.0], [0.5, 0.5, 0.5]];
        let colour = choose_colour(Some(colors), Some([1.0, 0.0, 0.5]));
        assert_eq!(
            colour,
            PartColour::PerVertex(vec![[1.0, 0.0, 0.5], [0.5, 0.0, 0.25]])
        );
    }

    #[test]
    fn a_material_without_vertex_colours_gives_a_flat_colour() {
        assert_eq!(
            choose_colour(None, Some([0.2, 0.4, 0.6])),
            PartColour::Flat([0.2, 0.4, 0.6])
        );
    }

    #[test]
    fn a_primitive_without_colour_uses_the_kind_table() {
        assert_eq!(choose_colour(None, None), PartColour::Table);
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
