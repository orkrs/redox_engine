//! One unit test per validation rule (EDT-B7 step 1).

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use redox_asset::AssetRegistry;
use redox_ecs::{World, set_parent};
use redox_editor_core::undo::UndoStack;
use redox_editor_scene::validate::{
    ValidateLevelOptions, ValidationFix, ValidationRule, ValidationSettings, apply_fix, rules,
    validate_level,
};
use redox_editor_scene::{LoadReport, LoadWarning, register_scene_components};
use redox_math::{Aabb, Transform, Vec3};
use redox_reflect::TypeRegistry;
use redox_render::Camera;
use redox_scene::{AssetKind, AssetRef, AssetRefs, EntityGuid, Name};

fn registry() -> TypeRegistry {
    let mut registry = TypeRegistry::new();
    register_scene_components(&mut registry);
    registry
}

fn temp_assets(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "redox_validate_{tag}_{}_{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn ctx<'a>(
    world: &'a World,
    registry: &'a TypeRegistry,
    assets: Option<&'a std::path::Path>,
    load: Option<&'a LoadReport>,
    asset_registry: Option<&'a AssetRegistry>,
    settings: &'a ValidationSettings,
) -> rules::RuleContext<'a> {
    rules::RuleContext {
        world,
        registry,
        assets_root: assets,
        asset_registry,
        reference_index: None,
        scene_path: None,
        settings,
        load_report: load,
    }
}

#[test]
fn missing_asset_rule_reports_a_broken_handle_path() {
    let dir = temp_assets("missing");
    let registry = registry();
    let mut world = World::new();
    let entity = world.spawn();
    let guid = EntityGuid::new_random();
    world.add_component(entity, guid);
    world.add_component(
        entity,
        AssetRefs(vec![AssetRef {
            component: "MeshHandle".into(),
            guid: None,
            path: Some("meshes/missing.glb".into()),
            kind: AssetKind::Mesh,
        }]),
    );
    let settings = ValidationSettings::default();
    let issues = rules::check_missing_and_broken_assets(&ctx(
        &world,
        &registry,
        Some(dir.as_path()),
        None,
        None,
        &settings,
    ));
    assert!(
        issues
            .iter()
            .any(|i| i.rule == ValidationRule::MissingAsset)
    );
}

#[test]
fn broken_asset_ref_rule_reports_an_unknown_guid() {
    let dir = temp_assets("broken_guid");
    let (asset_registry, _) = AssetRegistry::scan(&dir).expect("scan empty");
    let registry = registry();
    let mut world = World::new();
    let entity = world.spawn();
    let guid = EntityGuid::new_random();
    world.add_component(entity, guid);
    world.add_component(
        entity,
        AssetRefs(vec![AssetRef {
            component: "MaterialHandle".into(),
            guid: Some("not-a-valid-guid".into()),
            path: None,
            kind: AssetKind::Material,
        }]),
    );
    let settings = ValidationSettings::default();
    let issues = rules::check_missing_and_broken_assets(&ctx(
        &world,
        &registry,
        Some(dir.as_path()),
        None,
        Some(&asset_registry),
        &settings,
    ));
    assert!(
        issues
            .iter()
            .any(|i| i.rule == ValidationRule::BrokenAssetRef)
    );
}

#[test]
fn unknown_component_rule_maps_load_warnings() {
    let registry = registry();
    let world = World::new();
    let entity = EntityGuid::new_random();
    let report = LoadReport {
        warnings: vec![LoadWarning::UnknownComponent {
            entity,
            type_name: "LegacyLight".into(),
        }],
    };
    let settings = ValidationSettings::default();
    let issues = rules::check_unknown_components(&ctx(
        &world,
        &registry,
        None,
        Some(&report),
        None,
        &settings,
    ));
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].rule, ValidationRule::UnknownComponent);
}

#[test]
fn missing_parent_rule_reports_orphan_links() {
    let registry = registry();
    let world = World::new();
    let child = EntityGuid::new_random();
    let parent = EntityGuid::new_random();
    let report = LoadReport {
        warnings: vec![LoadWarning::MissingParent {
            entity: child,
            parent_guid: parent,
        }],
    };
    let settings = ValidationSettings::default();
    let issues = rules::check_hierarchy(&ctx(
        &world,
        &registry,
        None,
        Some(&report),
        None,
        &settings,
    ));
    assert!(
        issues
            .iter()
            .any(|i| i.rule == ValidationRule::MissingParent)
    );
}

#[test]
fn hierarchy_cycle_rule_detects_a_parent_loop() {
    let registry = registry();
    let mut world = World::new();
    let a = world.spawn();
    let b = world.spawn();
    let ga = EntityGuid::new_random();
    let gb = EntityGuid::new_random();
    world.add_component(a, ga);
    world.add_component(b, gb);
    set_parent(&mut world, a, b);
    set_parent(&mut world, b, a);
    let settings = ValidationSettings::default();
    let issues = rules::check_hierarchy(&ctx(&world, &registry, None, None, None, &settings));
    assert!(
        issues
            .iter()
            .any(|i| i.rule == ValidationRule::HierarchyCycle)
    );
}

#[test]
fn no_active_camera_rule_errors_when_there_is_no_camera() {
    let registry = registry();
    let world = World::new();
    let settings = ValidationSettings::default();
    let issues = rules::check_active_camera(&ctx(&world, &registry, None, None, None, &settings));
    assert!(
        issues
            .iter()
            .any(|i| i.rule == ValidationRule::NoActiveCamera)
    );
    assert!(issues[0].fix.is_some());
}

#[test]
fn texture_no_mips_rule_warns_on_a_large_png_without_meta() {
    let dir = temp_assets("tex");
    let png = dir.join("big.png");
    // minimal 2049x1 PNG would be huge; use 3000x4 stored as raw IHDR hack
    let mut bytes = vec![
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, b'I', b'H', b'D',
        b'R',
    ];
    let w: u32 = 3000;
    let h: u32 = 4;
    bytes.extend_from_slice(&w.to_be_bytes());
    bytes.extend_from_slice(&h.to_be_bytes());
    bytes.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit RGB
    bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // CRC placeholder (probe only reads IHDR)
    fs::write(&png, bytes).expect("write png");
    let (asset_registry, _) = AssetRegistry::scan(&dir).expect("scan");
    let registry = registry();
    let world = World::new();
    let settings = ValidationSettings {
        large_texture_threshold: 2048,
        ..ValidationSettings::default()
    };
    let ctx = rules::RuleContext {
        world: &world,
        registry: &registry,
        assets_root: Some(dir.as_path()),
        asset_registry: Some(&asset_registry),
        reference_index: None,
        scene_path: None,
        settings: &settings,
        load_report: None,
    };
    let issues = rules::check_texture_mips(&ctx);
    assert!(
        issues
            .iter()
            .any(|i| i.rule == ValidationRule::TextureNoMips)
    );
}

#[test]
fn out_of_world_bounds_rule_flags_a_far_transform() {
    let registry = registry();
    let mut world = World::new();
    let entity = world.spawn();
    world.add_component(entity, EntityGuid::new_random());
    world.add_component(
        entity,
        Transform::from_translation(Vec3::new(999_999.0, 0.0, 0.0)),
    );
    let settings = ValidationSettings {
        world_bounds: Aabb::from_center_size(Vec3::ZERO, Vec3::splat(100.0)),
        ..ValidationSettings::default()
    };
    let ctx = rules::RuleContext {
        world: &world,
        registry: &registry,
        assets_root: None,
        asset_registry: None,
        reference_index: None,
        scene_path: None,
        settings: &settings,
        load_report: None,
    };
    let issues = rules::check_world_bounds(&ctx);
    assert!(
        issues
            .iter()
            .any(|i| i.rule == ValidationRule::OutOfWorldBounds)
    );
}

#[test]
fn fix_on_a_scene_without_camera_adds_camera_in_one_undo_step() {
    let registry = registry();
    let mut world = World::new();
    let mut undo = UndoStack::default();
    apply_fix(
        &ValidationFix::AddDefaultCamera,
        &mut world,
        &registry,
        &mut undo,
    )
    .expect("fix");
    assert!(
        world
            .all_entities()
            .any(|e| world.get_component::<Camera>(e).is_some())
    );
    let undone = undo.undo(&mut world, &registry);
    assert!(undone, "undo removes the camera step");
}

#[test]
fn a_broken_asset_reference_produces_an_issue_with_select() {
    let dir = temp_assets("select");
    fs::create_dir_all(dir.join("meshes")).expect("meshes");
    let registry = registry();
    let mut world = World::new();
    let entity = world.spawn();
    let guid = EntityGuid::new_random();
    world.add_component(entity, guid);
    world.add_component(entity, Name("broken".into()));
    world.add_component(
        entity,
        AssetRefs(vec![AssetRef {
            component: "MeshHandle".into(),
            guid: None,
            path: Some("meshes/ghost.glb".into()),
            kind: AssetKind::Mesh,
        }]),
    );
    let report = validate_level(
        &world,
        &registry,
        ValidateLevelOptions {
            assets_root: Some(dir.as_path()),
            ..ValidateLevelOptions::default()
        },
    );
    let issue = report
        .issues
        .iter()
        .find(|i| i.rule == ValidationRule::MissingAsset)
        .expect("missing asset issue");
    assert_eq!(issue.select, Some(guid));
}
