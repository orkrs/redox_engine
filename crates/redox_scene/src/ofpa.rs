//! One File Per Actor layout: `world.ron` + `cells/<cx>_<cz>/<id>.ron`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::{PartitionCell, SceneResult};

pub const CELLS_DIR: &str = "cells";
pub const WORLD_FILE: &str = "world.ron";

fn default_schema_version() -> u32 {
    1
}

/// On-disk world header (`world.ron`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldManifest {
    #[serde(default = "default_schema_version")]
    pub version: u32,
    pub name: String,
    pub cell_size: f32,
    pub origin: [f32; 3],
    pub seed: u64,
    pub cells: Vec<PartitionCell>,
}

impl WorldManifest {
    pub fn new(name: impl Into<String>, cell_size: f32, seed: u64) -> Self {
        Self {
            version: 1,
            name: name.into(),
            cell_size,
            origin: [0.0; 3],
            seed,
            cells: Vec::new(),
        }
    }
}

/// One actor saved as a `.ron` file inside a cell directory.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActorRecord {
    pub id: String,
    pub kind: String,
    pub position: [f32; 3],
    #[serde(default)]
    pub yaw: f32,
    #[serde(default)]
    pub tags: Vec<String>,
}

pub fn world_ron(root: impl AsRef<Path>) -> PathBuf {
    root.as_ref().join(WORLD_FILE)
}

pub fn cell_dir(root: impl AsRef<Path>, cell: PartitionCell) -> PathBuf {
    root.as_ref()
        .join(CELLS_DIR)
        .join(format!("{}_{}", cell.x, cell.y))
}

pub fn actor_path(root: impl AsRef<Path>, cell: PartitionCell, actor_id: &str) -> PathBuf {
    cell_dir(root, cell).join(format!("{actor_id}.ron"))
}

/// Atomically writes content to `path` using a temporary file and atomic swap with backup.
pub fn write_atomic(path: impl AsRef<Path>, content: &[u8]) -> std::io::Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension(format!("tmp.{}", std::process::id()));
    fs::write(&tmp_path, content)?;

    if path.exists() {
        let bak_path = path.with_extension("bak");
        let _ = fs::copy(path, &bak_path);
        let _ = fs::remove_file(path);
    }

    fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Parses `cx_cz` (and the older `cell_cx_cz`) directory names.
pub fn parse_cell_dir(name: &str) -> Option<PartitionCell> {
    let rest = name.strip_prefix("cell_").unwrap_or(name);
    let (xs, ys) = rest.split_once('_')?;
    Some(PartitionCell {
        x: xs.parse().ok()?,
        y: ys.parse().ok()?,
    })
}

pub fn load_manifest(root: impl AsRef<Path>) -> SceneResult<WorldManifest> {
    let path = world_ron(&root);
    let text = fs::read_to_string(&path)?;
    Ok(ron::from_str(&text)?)
}

pub fn save_manifest(root: impl AsRef<Path>, manifest: &WorldManifest) -> SceneResult<()> {
    let root = root.as_ref();
    let pretty = ron::ser::PrettyConfig::new();
    let serialized = ron::ser::to_string_pretty(manifest, pretty)?;
    write_atomic(world_ron(root), serialized.as_bytes())?;
    Ok(())
}

pub fn load_cell_actors(
    root: impl AsRef<Path>,
    cell: PartitionCell,
) -> SceneResult<Vec<ActorRecord>> {
    let dir = cell_dir(root, cell);
    let mut actors = Vec::new();
    if !dir.is_dir() {
        return Ok(actors);
    }
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "ron") {
            let text = fs::read_to_string(&path)?;
            actors.push(ron::from_str(&text)?);
        }
    }
    Ok(actors)
}

pub fn save_cell_actors(
    root: impl AsRef<Path>,
    cell: PartitionCell,
    actors: &[ActorRecord],
) -> SceneResult<()> {
    let dir = cell_dir(&root, cell);
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    fs::create_dir_all(&dir)?;
    let pretty = ron::ser::PrettyConfig::new();
    for actor in actors {
        let serialized = ron::ser::to_string_pretty(actor, pretty.clone())?;
        write_atomic(actor_path(&root, cell, &actor.id), serialized.as_bytes())?;
    }
    Ok(())
}

pub fn save_actor(
    root: impl AsRef<Path>,
    cell: PartitionCell,
    actor: &ActorRecord,
) -> SceneResult<()> {
    let pretty = ron::ser::PrettyConfig::new();
    let serialized = ron::ser::to_string_pretty(actor, pretty)?;
    write_atomic(actor_path(root, cell, &actor.id), serialized.as_bytes())?;
    Ok(())
}

pub fn cell_mtime(root: impl AsRef<Path>, cell: PartitionCell) -> Option<SystemTime> {
    fs::metadata(cell_dir(root, cell))
        .and_then(|m| m.modified())
        .ok()
}

/// Writes a full OFPA world (manifest + every cell).
pub fn write_world(
    root: impl AsRef<Path>,
    mut manifest: WorldManifest,
    cells: &[(PartitionCell, Vec<ActorRecord>)],
) -> SceneResult<()> {
    let root = root.as_ref();
    fs::create_dir_all(root)?;
    manifest.cells = cells.iter().map(|(c, _)| *c).collect();
    save_manifest(root, &manifest)?;
    let cells_root = root.join(CELLS_DIR);
    if cells_root.exists() {
        fs::remove_dir_all(&cells_root)?;
    }
    for (cell, actors) in cells {
        save_cell_actors(root, *cell, actors)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_atomic_and_backup() {
        let temp_dir =
            std::env::temp_dir().join(format!("redox_atomic_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        let test_file = temp_dir.join("test.txt");

        // First write
        write_atomic(&test_file, b"version 1").unwrap();
        assert_eq!(fs::read_to_string(&test_file).unwrap(), "version 1");
        assert!(!test_file.with_extension("bak").exists());

        // Second write creates backup
        write_atomic(&test_file, b"version 2").unwrap();
        assert_eq!(fs::read_to_string(&test_file).unwrap(), "version 2");
        assert_eq!(
            fs::read_to_string(test_file.with_extension("bak")).unwrap(),
            "version 1"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_world_manifest_version() {
        let manifest = WorldManifest::new("TestWorld", 16.0, 42);
        assert_eq!(manifest.version, 1);
        let serialized = ron::to_string(&manifest).unwrap();
        let deserialized: WorldManifest = ron::from_str(&serialized).unwrap();
        assert_eq!(deserialized.version, 1);
    }
}
