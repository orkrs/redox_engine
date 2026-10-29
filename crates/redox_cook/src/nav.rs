//! `bake-navmesh`: glTF / GLB level geometry to a `.nav` navigation file.
//!
//! Every mesh primitive in the default scene graph is collected with its world transform
//! and baked as walkable ground. Non-triangle primitives are skipped.

use std::path::{Path, PathBuf};

use redox_math::Mat4;
use redox_nav::{AgentParams, NavAreaId, NavGeometry, bake, from_triangle_meshes};

/// Default tile size in metres.
const DEFAULT_TILE: f32 = 16.0;

/// A parsed `bake-navmesh` command.
#[derive(Clone, Debug, PartialEq)]
pub struct NavJob {
    /// glTF or GLB input.
    pub input: PathBuf,
    /// Output `.nav` file.
    pub out: PathBuf,
    /// Tile edge length in metres.
    pub tile: f32,
    /// Agent the mesh is built for.
    pub agent: AgentParams,
}

/// Parses the arguments after `bake-navmesh`.
///
/// `<input> <out.nav> [--tile M] [--radius M] [--height M] [--step M] [--slope DEG]`.
///
/// # Errors
///
/// Returns a message for missing arguments, unknown options, or values that are not numbers.
pub fn parse_bake_navmesh(args: &[&str]) -> Result<NavJob, String> {
    let [input, out, flags @ ..] = args else {
        return Err("bake-navmesh needs <input> <out.nav>".into());
    };
    let mut job = NavJob {
        input: input.into(),
        out: out.into(),
        tile: DEFAULT_TILE,
        agent: AgentParams::default(),
    };
    let mut index = 0;
    while index < flags.len() {
        let flag = flags[index];
        let value = flags
            .get(index + 1)
            .copied()
            .ok_or_else(|| format!("{flag} needs a value"))?;
        let number: f32 = value
            .parse()
            .map_err(|_| format!("{flag}: not a number: {value}"))?;
        match flag {
            "--tile" => job.tile = number,
            "--radius" => job.agent.radius = number,
            "--height" => job.agent.height = number,
            "--step" => job.agent.max_step = number,
            "--slope" => job.agent.max_slope_deg = number,
            other => return Err(format!("unknown option: {other}")),
        }
        index += 2;
    }
    Ok(job)
}

/// Runs the bake and writes the `.nav` file.
///
/// # Errors
///
/// Returns a message when the input cannot be read, the bake fails, or the output cannot be written.
pub fn run(job: &NavJob) -> Result<(), String> {
    let geometry = read_geometry(&job.input)?;
    let data = bake(&geometry, &job.agent, job.tile).map_err(|e| e.to_string())?;
    let bytes = data.to_bytes();
    std::fs::write(&job.out, &bytes)
        .map_err(|e| format!("cannot write {}: {e}", job.out.display()))?;
    println!(
        "{}: {} tiles, {} bytes",
        job.out.display(),
        data.tiles.len(),
        bytes.len()
    );
    Ok(())
}

/// One mesh primitive with its world transform, owned so the borrows of [`from_triangle_meshes`] work.
struct MeshPart {
    positions: Vec<[f32; 3]>,
    indices: Vec<u32>,
    transform: Mat4,
}

fn read_geometry(path: &Path) -> Result<NavGeometry, String> {
    let (document, buffers, _) =
        gltf::import(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut parts = Vec::new();
    for scene in document.scenes() {
        for node in scene.nodes() {
            collect_node(&node, Mat4::IDENTITY, &buffers, &mut parts);
        }
    }
    Ok(from_triangle_meshes(parts.iter().map(|part| {
        (
            part.positions.as_slice(),
            part.indices.as_slice(),
            part.transform,
            NavAreaId::WALKABLE,
        )
    })))
}

fn collect_node(
    node: &gltf::Node<'_>,
    parent: Mat4,
    buffers: &[gltf::buffer::Data],
    parts: &mut Vec<MeshPart>,
) {
    let local = Mat4::from_cols_array_2d(&node.transform().matrix());
    let world = parent * local;
    if let Some(mesh) = node.mesh() {
        for primitive in mesh.primitives() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(|b| &b.0[..]));
            let Some(positions) = reader.read_positions() else {
                continue;
            };
            let positions: Vec<[f32; 3]> = positions.collect();
            let indices: Vec<u32> = match reader.read_indices() {
                Some(indices) => indices.into_u32().collect(),
                None => (0..positions.len() as u32).collect(),
            };
            parts.push(MeshPart {
                positions,
                indices,
                transform: world,
            });
        }
    }
    for child in node.children() {
        collect_node(&child, world, buffers, parts);
    }
}
