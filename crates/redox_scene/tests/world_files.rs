//! OFPA files, procedural generation and streaming against a real temp directory.

use std::path::PathBuf;

use redox_ecs::World;
use redox_math::Vec3;
use redox_scene::ofpa::{ActorRecord, WorldManifest, load_cell_actors, load_manifest, save_actor};
use redox_scene::{
    PartitionCell, PcgKind, PcgParams, StreamedActor, WorldPartition, generate, generate_to,
};

/// A fresh, empty directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("redox_scene_{tag}_{}", std::process::id()));
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

#[test]
fn cell_lookup_floors_towards_negative_infinity() {
    let c = PartitionCell::from_world(16.0, [0.0; 3], [15.9, 0.0, -0.1]);
    assert_eq!((c.x, c.y), (0, -1));
    let c = PartitionCell::from_world(16.0, [0.0; 3], [-16.0, 0.0, 16.0]);
    assert_eq!((c.x, c.y), (-1, 1));
    let shifted = PartitionCell::from_world(10.0, [100.0, 0.0, 100.0], [105.0, 0.0, 95.0]);
    assert_eq!((shifted.x, shifted.y), (0, -1));
}

#[test]
fn generation_is_deterministic_for_a_seed() {
    let params = PcgParams {
        kind: PcgKind::Dungeon,
        seed: 42,
        ..Default::default()
    };
    let a = generate(&params);
    let b = generate(&params);
    assert_eq!(a.cells.len(), b.cells.len());
    for ((ca, aa), (cb, ab)) in a.cells.iter().zip(&b.cells) {
        assert_eq!(ca, cb);
        assert_eq!(aa, ab);
    }
}

#[test]
fn manifest_and_actors_round_trip_through_disk() {
    let dir = TempDir::new("roundtrip");
    let params = PcgParams {
        kind: PcgKind::Arena,
        width: 2,
        height: 2,
        ..Default::default()
    };
    let world = generate_to(&dir.0, &params).unwrap();

    let manifest = load_manifest(&dir.0).unwrap();
    assert_eq!(manifest.cells.len(), world.cells.len());

    let (cell, expected) = &world.cells[0];
    let mut loaded = load_cell_actors(&dir.0, *cell).unwrap();
    let mut expected = expected.clone();
    loaded.sort_by(|a, b| a.id.cmp(&b.id));
    expected.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(loaded, expected);

    let extra = ActorRecord {
        id: "extra".into(),
        kind: "prop".into(),
        position: [1.0, 2.0, 3.0],
        yaw: 0.5,
        tags: vec!["test".into()],
    };
    save_actor(&dir.0, *cell, &extra).unwrap();
    let again = load_cell_actors(&dir.0, *cell).unwrap();
    assert!(again.contains(&extra));
}

#[test]
fn missing_manifest_is_an_error_not_a_panic() {
    let dir = TempDir::new("missing");
    assert!(load_manifest(&dir.0).is_err());
    assert!(WorldPartition::open(&dir.0).is_err());
}

#[test]
fn partition_streams_cells_around_the_viewer() {
    let mut cells = std::collections::HashMap::new();
    for x in -3..=3 {
        cells.insert(
            PartitionCell { x, y: 0 },
            vec![ActorRecord {
                id: format!("a{x}"),
                kind: "prop".into(),
                position: [x as f32 * 16.0 + 1.0, 0.0, 1.0],
                yaw: 0.0,
                tags: vec![],
            }],
        );
    }
    let mut part = WorldPartition::in_memory(16.0, [0.0; 3], cells);
    part.config.budget.max_loads = 16;
    let mut world = World::new();

    for _ in 0..4 {
        part.tick(&mut world, Vec3::new(0.5, 0.0, 0.5));
    }
    assert!(part.is_loaded(PartitionCell { x: 0, y: 0 }));
    assert!(part.is_loaded(PartitionCell { x: 1, y: 0 }));
    assert!(
        !part.is_loaded(PartitionCell { x: 3, y: 0 }),
        "outside the load radius"
    );
    assert!(part.loaded_entity_count() > 0);
    let streamed = world.query::<&StreamedActor>().iter(&world).count();
    assert_eq!(streamed, part.loaded_entity_count());

    // Move far away: the first cells unload once outside the unload radius.
    for _ in 0..8 {
        part.tick(&mut world, Vec3::new(3.0 * 16.0 + 0.5, 0.0, 0.5));
    }
    assert!(!part.is_loaded(PartitionCell { x: 0, y: 0 }));
    assert!(part.is_loaded(PartitionCell { x: 3, y: 0 }));
}

#[test]
fn new_manifest_has_schema_version_one() {
    assert_eq!(WorldManifest::new("w", 8.0, 1).version, 1);
}
