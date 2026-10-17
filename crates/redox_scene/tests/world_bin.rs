//! Binary world files: round trip, OFPA conversion, corruption handling.

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use redox_scene::PartitionCell;
use redox_scene::ofpa::{ActorRecord, WorldManifest, write_world};
use redox_scene::world_bin::{BinWorld, convert_ofpa_to_bin, world_to_bytes, write_world_bin};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("redox_wbin_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn actor(id: &str, x: f32) -> ActorRecord {
    ActorRecord {
        id: id.into(),
        kind: "tree".into(),
        position: [x, 0.0, 1.0],
        yaw: 0.5,
        tags: vec!["a".into(), "b".into()],
    }
}

fn sample_cells() -> Vec<(PartitionCell, Vec<ActorRecord>)> {
    vec![
        (
            PartitionCell { x: 0, y: 0 },
            vec![actor("t1", 1.0), actor("t2", 2.0)],
        ),
        (PartitionCell { x: -3, y: 7 }, vec![actor("far", 99.0)]),
        (PartitionCell { x: 1, y: 1 }, vec![]),
    ]
}

#[test]
fn round_trip_keeps_manifest_cells_and_actors() {
    let dir = TempDir::new("rt");
    let path = dir.0.join("world.bin");
    let cells = sample_cells();
    write_world_bin(&path, WorldManifest::new("Test", 32.0, 7), &cells).unwrap();

    let mut world = BinWorld::open(&path).unwrap();
    assert_eq!(world.manifest().name, "Test");
    assert_eq!(world.manifest().cell_size, 32.0);
    assert_eq!(
        world.manifest().cells.len(),
        3,
        "manifest lists the stored cells"
    );
    assert_eq!(world.cells().count(), 3);
    for (cell, actors) in &cells {
        assert_eq!(&world.load_cell(*cell).unwrap(), actors);
    }
    assert!(
        world
            .load_cell(PartitionCell { x: 50, y: 50 })
            .unwrap()
            .is_empty()
    );
}

#[test]
fn conversion_from_ofpa_matches_the_source_tree() {
    let dir = TempDir::new("conv");
    let root = dir.0.join("world");
    write_world(&root, WorldManifest::new("W", 16.0, 1), &sample_cells()).unwrap();
    let bin = dir.0.join("w.bin");
    convert_ofpa_to_bin(&root, &bin).unwrap();

    let mut world = BinWorld::open(&bin).unwrap();
    let mut loaded = world.load_cell(PartitionCell { x: 0, y: 0 }).unwrap();
    loaded.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(loaded, vec![actor("t1", 1.0), actor("t2", 2.0)]);
}

#[test]
fn binary_form_is_deterministic_and_smaller_than_text() {
    let cells: Vec<_> = (0..20)
        .map(|i| {
            (
                PartitionCell { x: i, y: 0 },
                (0..50)
                    .map(|j| actor(&format!("a{i}_{j}"), j as f32))
                    .collect(),
            )
        })
        .collect();
    let a = world_to_bytes(WorldManifest::new("W", 8.0, 3), &cells).unwrap();
    let b = world_to_bytes(WorldManifest::new("W", 8.0, 3), &cells).unwrap();
    assert_eq!(a, b);

    let ron_size: usize = cells
        .iter()
        .flat_map(|(_, actors)| actors)
        .map(|a| {
            ron::ser::to_string_pretty(a, ron::ser::PrettyConfig::new())
                .unwrap()
                .len()
        })
        .sum();
    assert!(a.len() * 3 < ron_size, "{} vs {ron_size}", a.len());
}

#[test]
fn corruption_is_reported_never_panics() {
    let dir = TempDir::new("bad");
    let path = dir.0.join("w.bin");
    let bytes = world_to_bytes(WorldManifest::new("W", 8.0, 3), &sample_cells()).unwrap();

    for i in 0..bytes.len() {
        let mut broken = bytes.clone();
        broken[i] ^= 0x5A;
        std::fs::write(&path, &broken).unwrap();
        if let Ok(mut world) = BinWorld::open(&path) {
            // A flipped byte must be caught by a checksum or leave the data intact.
            for cell in world.cells().collect::<Vec<_>>() {
                let _ = world.load_cell(cell);
            }
        }
    }
    for cut in 0..bytes.len() {
        std::fs::write(&path, &bytes[..cut]).unwrap();
        if let Ok(mut world) = BinWorld::open(&path) {
            for cell in world.cells().collect::<Vec<_>>() {
                let _ = world.load_cell(cell);
            }
        }
    }
    std::fs::write(&path, b"RDXWLD1\0garbage").unwrap();
    assert!(BinWorld::open(&path).is_err());
    assert!(BinWorld::open(dir.0.join("missing.bin")).is_err());
}

#[test]
fn flipped_cell_byte_is_detected_by_the_checksum() {
    let dir = TempDir::new("flip");
    let path = dir.0.join("w.bin");
    let mut bytes = world_to_bytes(WorldManifest::new("W", 8.0, 3), &sample_cells()).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    std::fs::write(&path, &bytes).unwrap();
    let mut world = BinWorld::open(&path).unwrap();
    let results: Vec<_> = world
        .cells()
        .collect::<Vec<_>>()
        .into_iter()
        .map(|c| world.load_cell(c))
        .collect();
    assert!(results.iter().any(Result::is_err));
}
