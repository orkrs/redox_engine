//! Headless tests of `SceneDocument`: save, open, identity, warnings, versions. No GPU.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use redox_ecs::{Children, Entity, Parent, World, set_parent};
use redox_editor_scene::format::{SCENE_FORMAT_VERSION, SceneEntity, SceneFile};
use redox_editor_scene::{LoadWarning, SceneDocument, SceneError, register_scene_components};
use redox_math::{Transform, Vec3};
use redox_reflect::{Reflect, ReflectValue, TypeRegistry};
use redox_render::{Camera, DirectionalLight, MaterialHandle, MeshHandle, PointLight, SpotLight};
use redox_scene::{EntityGuid, Name, ParentRef, Tag};

/// Components, parent guid and sorted child guids of every entity with a guid.
type Described = (Vec<ReflectValue>, Option<EntityGuid>, Vec<EntityGuid>);

fn registry() -> TypeRegistry {
    let mut registry = TypeRegistry::new();
    register_scene_components(&mut registry);
    registry
}

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "redox_scene_doc_{tag}_{}_{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn write_scene(path: &Path, file: &SceneFile) -> String {
    let text = ron::ser::to_string_pretty(file, ron::ser::PrettyConfig::default())
        .expect("serialize test scene");
    fs::write(path, &text).expect("write test scene");
    text
}

fn scene_with(entities: Vec<SceneEntity>) -> SceneFile {
    SceneFile {
        format_version: SCENE_FORMAT_VERSION,
        name: "test".into(),
        entities,
    }
}

/// Root A with child B, which has child C; every entity has several component types.
fn build_hierarchy(world: &mut World) -> (Entity, Entity, Entity) {
    let a = world.spawn();
    world.add_component(a, Name("root".into()));
    world.add_component(a, Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)));
    world.add_component(
        a,
        DirectionalLight::new(Vec3::new(0.0, 1.0, 0.0), Vec3::ONE, 2.0),
    );
    world.add_component(a, Camera::default());

    let b = world.spawn();
    world.add_component(b, Name("mid".into()));
    world.add_component(b, Tag("group".into()));
    world.add_component(
        b,
        PointLight::new(Vec3::new(4.0, 5.0, 6.0), Vec3::ONE, 3.0, 9.0),
    );
    world.add_component(b, MeshHandle::default());

    let c = world.spawn();
    world.add_component(c, Name("leaf".into()));
    world.add_component(c, SpotLight::default());
    world.add_component(c, MaterialHandle::default());

    set_parent(world, b, a);
    set_parent(world, c, b);
    (a, b, c)
}

/// Describes every entity that has a guid, keyed by guid, so that worlds with different
/// entity handles can be compared.
fn describe(world: &World, registry: &TypeRegistry) -> HashMap<EntityGuid, Described> {
    let mut out = HashMap::new();
    for entity in world.all_entities() {
        let Some(guid) = world.get_component::<EntityGuid>(entity).copied() else {
            continue;
        };
        let components = registry.snapshot_entity(world, entity);
        let parent = world
            .get_component::<Parent>(entity)
            .and_then(|parent| world.get_component::<EntityGuid>(parent.0).copied());
        let mut children: Vec<EntityGuid> = world
            .get_component::<Children>(entity)
            .map(|kids| {
                kids.list
                    .iter()
                    .filter_map(|child| world.get_component::<EntityGuid>(*child).copied())
                    .collect()
            })
            .unwrap_or_default();
        children.sort();
        out.insert(guid, (components, parent, children));
    }
    out
}

#[test]
fn a_saved_scene_loads_back_with_the_same_components_guids_and_hierarchy() {
    let dir = temp_dir("roundtrip");
    let path = dir.join("main.scene.ron");
    let registry = registry();
    let mut world = World::new();
    build_hierarchy(&mut world);
    let mut document = SceneDocument::new("main");
    document
        .save_as(&path, &registry, &mut world)
        .expect("save");
    let saved = describe(&world, &registry);

    let mut loaded_world = World::new();
    let (loaded, report) =
        SceneDocument::open(&path, &registry, &mut loaded_world).expect("open");

    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert_eq!(loaded.name, "main");
    assert_eq!(describe(&loaded_world, &registry), saved);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn saving_the_same_world_twice_writes_the_same_bytes() {
    let dir = temp_dir("twice");
    let first = dir.join("first.scene.ron");
    let second = dir.join("second.scene.ron");
    let registry = registry();
    let mut world = World::new();
    build_hierarchy(&mut world);
    let mut document = SceneDocument::new("main");

    document
        .save_as(&first, &registry, &mut world)
        .expect("first save");
    document
        .save_as(&second, &registry, &mut world)
        .expect("second save");

    assert_eq!(
        fs::read(&first).expect("read first"),
        fs::read(&second).expect("read second")
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn opening_and_saving_again_writes_the_same_bytes() {
    let dir = temp_dir("reopen");
    let first = dir.join("first.scene.ron");
    let second = dir.join("second.scene.ron");
    let registry = registry();
    let mut world = World::new();
    build_hierarchy(&mut world);
    SceneDocument::new("main")
        .save_as(&first, &registry, &mut world)
        .expect("save");

    let mut reopened = World::new();
    let (mut document, _) =
        SceneDocument::open(&first, &registry, &mut reopened).expect("open");
    document
        .save_as(&second, &registry, &mut reopened)
        .expect("save again");

    assert_eq!(
        fs::read(&first).expect("read first"),
        fs::read(&second).expect("read second")
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn saving_gives_entities_without_a_guid_one() {
    let dir = temp_dir("assign");
    let path = dir.join("main.scene.ron");
    let registry = registry();
    let mut world = World::new();
    let entity = world.spawn();
    world.add_component(entity, Name("plain".into()));

    SceneDocument::new("main")
        .save_as(&path, &registry, &mut world)
        .expect("save");

    assert!(world.get_component::<EntityGuid>(entity).is_some());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_damaged_file_fails_with_a_line_number_and_is_left_unchanged() {
    let dir = temp_dir("damaged");
    let path = dir.join("main.scene.ron");
    let broken = "SceneFile(\n    format_version: 1,\n    name: \"broken\",\n    entities: [\n        (guid: Some(\"0123\"), components: [ oops ]\n";
    fs::write(&path, broken).expect("write damaged file");
    let mut world = World::new();

    let error = SceneDocument::open(&path, &registry(), &mut world)
        .err()
        .expect("a damaged file must not open");

    match error {
        SceneError::Parse { line, .. } => assert!(line >= 1, "line starts at 1"),
        other => panic!("expected a Parse error, got {other:?}"),
    }
    assert_eq!(fs::read_to_string(&path).expect("read back"), broken);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_file_from_a_newer_editor_is_refused() {
    let dir = temp_dir("newer");
    let path = dir.join("main.scene.ron");
    write_scene(
        &path,
        &SceneFile {
            format_version: 99,
            name: "future".into(),
            entities: Vec::new(),
        },
    );
    let mut world = World::new();

    let error = SceneDocument::open(&path, &registry(), &mut world)
        .err()
        .expect("a newer file must not open");

    assert!(matches!(
        error,
        SceneError::NewerFormat {
            found: 99,
            supported: SCENE_FORMAT_VERSION
        }
    ));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_unknown_component_is_reported_and_written_back_on_save() {
    let dir = temp_dir("unknown");
    let path = dir.join("main.scene.ron");
    let guid: EntityGuid = "0123456789abcdef0123456789abcdef"
        .parse()
        .expect("valid guid");
    let gizmo = ReflectValue::Struct {
        type_name: "Gizmo".into(),
        fields: vec![("x".into(), ReflectValue::F64(1.0))],
    };
    write_scene(
        &path,
        &scene_with(vec![SceneEntity {
            guid: Some(guid),
            components: Vec::new(),
            unknown_components: vec![gizmo.clone()],
        }]),
    );
    let mut world = World::new();
    let (mut document, report) =
        SceneDocument::open(&path, &registry(), &mut world).expect("open");

    assert_eq!(
        report.warnings,
        vec![LoadWarning::UnknownComponent {
            entity: guid,
            type_name: "Gizmo".into(),
        }]
    );
    let copy = dir.join("copy.scene.ron");
    document
        .save_as(&copy, &registry(), &mut world)
        .expect("save");
    let text = fs::read_to_string(&copy).expect("read copy");
    let written: SceneFile = ron::from_str(&text).expect("parse copy");
    assert_eq!(written.entities.len(), 1);
    assert_eq!(written.entities[0].guid, Some(guid));
    assert_eq!(written.entities[0].unknown_components, vec![gizmo]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_leftover_temporary_file_does_not_stop_opening() {
    let dir = temp_dir("leftover");
    let path = dir.join("main.scene.ron");
    let registry = registry();
    let mut world = World::new();
    build_hierarchy(&mut world);
    SceneDocument::new("main")
        .save_as(&path, &registry, &mut world)
        .expect("save");
    fs::write(dir.join("main.scene.tmp.999"), "half written").expect("write leftover");

    let mut reopened = World::new();
    let (_, report) = SceneDocument::open(&path, &registry, &mut reopened).expect("open");

    assert!(report.warnings.is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_handle_to_a_missing_asset_is_reported() {
    let dir = temp_dir("asset");
    let path = dir.join("main.scene.ron");
    let assets = dir.join("assets");
    fs::create_dir_all(&assets).expect("create assets root");
    let guid: EntityGuid = "fedcba9876543210fedcba9876543210"
        .parse()
        .expect("valid guid");
    write_scene(
        &path,
        &scene_with(vec![SceneEntity {
            guid: Some(guid),
            components: vec![ReflectValue::handle(
                "MeshHandle",
                Some("models/missing.glb".into()),
            )],
            unknown_components: Vec::new(),
        }]),
    );
    let mut world = World::new();

    let (_, report) = SceneDocument::open_with_assets(
        &path,
        &registry(),
        &mut world,
        Some(assets.as_path()),
    )
    .expect("open");

    assert_eq!(
        report.warnings,
        vec![LoadWarning::MissingAsset {
            entity: guid,
            type_name: "MeshHandle".into(),
            path: "models/missing.glb".into(),
        }]
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_parent_guid_that_is_not_in_the_file_is_reported() {
    let dir = temp_dir("parent");
    let path = dir.join("main.scene.ron");
    let child: EntityGuid = "00000000000000000000000000000001"
        .parse()
        .expect("valid guid");
    let missing: EntityGuid = "00000000000000000000000000000002"
        .parse()
        .expect("valid guid");
    write_scene(
        &path,
        &scene_with(vec![SceneEntity {
            guid: Some(child),
            components: vec![
                Name("orphan".into()).to_value(),
                ParentRef(missing).to_value(),
            ],
            unknown_components: Vec::new(),
        }]),
    );
    let mut world = World::new();

    let (_, report) = SceneDocument::open(&path, &registry(), &mut world).expect("open");

    assert_eq!(
        report.warnings,
        vec![LoadWarning::MissingParent {
            entity: child,
            parent_guid: missing,
        }]
    );
    let loaded = world.all_entities().next().expect("the child is loaded");
    assert!(world.get_component::<Parent>(loaded).is_none());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_entity_without_a_guid_gets_one_and_is_reported() {
    let dir = temp_dir("no_guid");
    let path = dir.join("main.scene.ron");
    write_scene(
        &path,
        &scene_with(vec![SceneEntity {
            guid: None,
            components: vec![Name("nameless".into()).to_value()],
            unknown_components: Vec::new(),
        }]),
    );
    let mut world = World::new();

    let (_, report) = SceneDocument::open(&path, &registry(), &mut world).expect("open");

    assert_eq!(report.warnings, vec![LoadWarning::MissingGuid { entity: 0 }]);
    let loaded = world.all_entities().next().expect("the entity is loaded");
    assert!(world.get_component::<EntityGuid>(loaded).is_some());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn two_entities_with_the_same_guid_are_refused() {
    let dir = temp_dir("duplicate");
    let path = dir.join("main.scene.ron");
    let guid = EntityGuid::new_random();
    write_scene(
        &path,
        &scene_with(vec![
            SceneEntity {
                guid: Some(guid),
                components: Vec::new(),
                unknown_components: Vec::new(),
            },
            SceneEntity {
                guid: Some(guid),
                components: Vec::new(),
                unknown_components: Vec::new(),
            },
        ]),
    );
    let mut world = World::new();

    let error = SceneDocument::open(&path, &registry(), &mut world)
        .err()
        .expect("duplicate guids must not open");

    assert!(matches!(error, SceneError::Invalid { .. }));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_version_zero_file_is_migrated_and_its_old_copy_is_kept() {
    let dir = temp_dir("migrate");
    let path = dir.join("main.scene.ron");
    let original = write_scene(
        &path,
        &SceneFile {
            format_version: 0,
            name: "old".into(),
            entities: Vec::new(),
        },
    );
    let registry = registry();
    let mut world = World::new();
    let (mut document, _) = SceneDocument::open(&path, &registry, &mut world).expect("open");
    assert_eq!(document.format_version, SCENE_FORMAT_VERSION);

    document.save(&registry, &mut world).expect("save");

    let backup = dir.join("main.scene.ron.bak-v0");
    assert_eq!(fs::read_to_string(&backup).expect("backup exists"), original);
    let written: SceneFile =
        ron::from_str(&fs::read_to_string(&path).expect("read")).expect("parse");
    assert_eq!(written.format_version, SCENE_FORMAT_VERSION);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn saving_without_a_file_asks_for_save_as() {
    let mut document = SceneDocument::new("untitled");
    let mut world = World::new();

    let error = document
        .save(&registry(), &mut world)
        .expect_err("no file yet");

    assert!(matches!(error, SceneError::Invalid { .. }));
}
