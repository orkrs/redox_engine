//! Tests of `Level`: opening scene files and world folders, unsupported paths and the dirty state.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use redox_ecs::World;
use redox_editor_scene::{Level, LevelError, LevelKind, WorldSession};
use redox_math::{Transform, Vec3};
use redox_reflect::{TypeRegistry, register_engine_defaults};
use redox_scene::PartitionCell;
use redox_scene::ofpa::{self, ActorRecord};

/// A unique folder under the system temp directory, removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "redox_level_{tag}_{}_{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn registry() -> TypeRegistry {
    let mut registry = TypeRegistry::new();
    register_engine_defaults(&mut registry);
    registry
}

#[test]
fn scene_file_opens_as_a_scene_level() {
    let dir = TempDir::new("scene_open");
    let registry = registry();
    let mut world = World::new();
    let mut level = Level::new_scene("main", &registry, &world);
    let file = dir.path().join("main.scene.ron");
    level
        .save_as(&file, &registry, &mut world)
        .expect("save scene");

    let mut loaded_world = World::new();
    let (opened, _report) =
        Level::open(&file, &registry, &mut loaded_world).expect("open scene file");

    assert!(!opened.is_world());
    assert!(matches!(opened.kind(), LevelKind::Scene(_)));
    assert_eq!(opened.title(), "main");
    assert_eq!(opened.path(), Some(file.as_path()));
}

#[test]
fn world_folder_opens_as_a_world_level() {
    let dir = TempDir::new("world_open");
    let root = dir.path().join("arena");
    WorldSession::create_new(&root, "empty", 4, 4).expect("create world");
    let actor = ActorRecord {
        id: "crate_0".to_string(),
        kind: "crate".to_string(),
        position: [1.0, 0.0, 1.0],
        yaw: 0.0,
        tags: Vec::new(),
    };
    ofpa::save_actor(&root, PartitionCell { x: 0, y: 0 }, &actor).expect("write one actor");

    let registry = registry();
    let mut world = World::new();
    let (opened, _report) = Level::open(&root, &registry, &mut world).expect("open world");

    assert!(opened.is_world());
    assert!(matches!(opened.kind(), LevelKind::World(_)));
    assert_eq!(opened.path(), Some(root.as_path()));
}

#[test]
fn unsupported_paths_are_refused() {
    let dir = TempDir::new("unsupported");
    let registry = registry();
    let mut world = World::new();

    let text_file = dir.path().join("notes.txt");
    fs::write(&text_file, "not a level").expect("write text file");
    let plain_folder = dir.path().join("plain");
    fs::create_dir_all(&plain_folder).expect("create plain folder");

    for path in [&text_file, &plain_folder] {
        let err = match Level::open(path, &registry, &mut world) {
            Ok(_) => panic!("path {} must be refused", path.display()),
            Err(err) => err,
        };
        assert!(matches!(err, LevelError::Unsupported { .. }));
    }
}

#[test]
fn dirty_state_follows_transform_edits_and_resets_on_save() {
    let dir = TempDir::new("dirty");
    let registry = registry();
    let mut world = World::new();
    let mut level = Level::new_scene("dirty", &registry, &world);
    assert!(!level.is_dirty(&registry, &world));

    let entity = world.spawn();
    world.add_component(entity, Transform::default());
    assert!(level.is_dirty(&registry, &world), "spawning must dirty the level");

    let file = dir.path().join("dirty.scene.ron");
    level
        .save_as(&file, &registry, &mut world)
        .expect("save scene");
    assert!(!level.is_dirty(&registry, &world));

    world
        .get_component_mut::<Transform>(entity)
        .expect("transform present")
        .translation = Vec3::new(3.0, 0.0, 0.0);
    assert!(level.is_dirty(&registry, &world), "editing Transform must dirty the level");

    level.save(&registry, &mut world).expect("save again");
    assert!(!level.is_dirty(&registry, &world));
}
