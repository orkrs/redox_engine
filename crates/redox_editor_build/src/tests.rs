//! Tests for build profiles, cook plan, snapshot and validation (EDT-B8/C1).

use std::fs;
use std::path::PathBuf;

use redox_asset::AssetRegistry;
use redox_ecs::World;
use redox_editor_core::editor_only::EditorOnly;
use redox_editor_core::project::{
    BuildProfile, BuildTarget, MANIFEST_FILE, ProjectContext, ProjectManifest,
};
use redox_editor_core::project_template::ProjectTemplate;
use redox_editor_scene::register_scene_components;
use redox_math::{Transform, Vec3};
use redox_reflect::TypeRegistry;
use redox_scene::Name;
use redox_scene::components::EntityGuid;

use crate::InclusionReason;
use crate::plan::build_cook_plan;
use crate::service::BuildService;
use crate::snapshot::BuildSnapshot;
use crate::validate::{ValidationSeverity, validate_project};

fn temp(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|t| t.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("redox_build_{tag}_{nanos}"))
}

#[test]
fn manifest_roundtrip_with_two_profiles() {
    let dir = temp("profiles");
    let _ = fs::remove_dir_all(&dir);
    ProjectContext::create(&dir, "Cook", ProjectTemplate::Empty).expect("create project");
    let path = dir.join(MANIFEST_FILE);
    let mut manifest: ProjectManifest =
        ron::from_str(&fs::read_to_string(&path).expect("read")).expect("parse");
    manifest.build_profiles = vec![
        BuildProfile {
            name: "release".into(),
            target: BuildTarget::Pak,
            compress: true,
            include_debug_assets: false,
            scene: None,
        },
        BuildProfile {
            name: "debug".into(),
            target: BuildTarget::Pak,
            compress: false,
            include_debug_assets: true,
            scene: Some("scenes/main.scene.ron".into()),
        },
    ];
    let text = ron::ser::to_string_pretty(&manifest, ron::ser::PrettyConfig::default())
        .expect("serialize");
    fs::write(&path, text).expect("write manifest");
    let opened = ProjectContext::open(&dir).expect("reopen");
    assert_eq!(opened.manifest().build_profiles.len(), 2);
    assert_eq!(opened.manifest().build_profiles[1].name, "debug");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn cook_plan_includes_shared_texture_and_skips_unused() {
    let dir = temp("plan");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("assets/textures")).expect("assets");
    fs::create_dir_all(dir.join("scenes")).expect("scenes");
    fs::write(
        dir.join("assets/textures/shared.png"),
        crate::test_support::MIN_PNG,
    )
    .expect("png");
    fs::write(
        dir.join("assets/textures/unused.png"),
        crate::test_support::MIN_PNG,
    )
    .expect("png");
    let (registry, _) = AssetRegistry::scan(dir.join("assets")).expect("scan");
    let shared = registry
        .guid_of("textures/shared.png")
        .expect("shared guid");
    let shared_text = format!("{:032x}", shared.0);
    let scene = dir.join("scenes/main.scene.ron");
    fs::write(
        &scene,
        format!(
            r#"(
    format_version: 1,
    name: "Plan",
    entities: [
        (
            guid: Some("00000000000000000000000000000001"),
            components: [
                Struct(type_name: "MaterialHandle", fields: [("guid", String("{shared_text}")), ("path", String("textures/shared.png"))]),
            ],
        ),
        (
            guid: Some("00000000000000000000000000000002"),
            components: [
                Struct(type_name: "MaterialHandle", fields: [("guid", String("{shared_text}")), ("path", String("textures/shared.png"))]),
            ],
        ),
    ],
)"#
        ),
    )
    .expect("scene");
    fs::write(
        dir.join("project.ron"),
        r#"(
    format_version: 1,
    name: "Plan",
    engine_version: "0.1.0",
    start_scene: "scenes/main.scene.ron",
    assets_root: "assets",
    plugins: [],
    build_profiles: [],
    created: "2026-01-01T00:00:00Z",
    last_opened: "2026-01-01T00:00:00Z",
)"#,
    )
    .expect("manifest");
    let profile = BuildProfile::release_default();
    let plan = build_cook_plan(&dir, &registry, &profile, &scene).expect("plan");
    assert!(plan.assets.len() >= 1, "{:?}", plan.assets);
    let shared = registry
        .guid_of("textures/shared.png")
        .expect("shared guid");
    assert!(plan.contains(shared));
    let unused_guid = registry
        .guid_of("textures/unused.png")
        .expect("unused guid");
    assert!(!plan.contains(unused_guid));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn editor_only_entity_is_absent_from_build_snapshot() {
    let mut registry = TypeRegistry::new();
    register_scene_components(&mut registry);
    let mut world = World::new();
    let e = world.spawn();
    world.add_component(e, Name("Ship".into()));
    world.add_component(e, Transform::from_translation(Vec3::ZERO));
    world.add_component(e, EntityGuid(1));
    let gizmo = world.spawn();
    world.add_component(gizmo, EditorOnly);
    world.add_component(gizmo, Name("Gizmo".into()));
    let snap = BuildSnapshot::from_world(&registry, &world);
    assert_eq!(snap.entity_count(), 1);
    assert!(!BuildSnapshot::is_stripped(&world, e));
    assert!(BuildSnapshot::is_stripped(&world, gizmo));
}

#[test]
fn cook_report_lists_reason_per_guid() {
    let dir = temp("cook");
    let _ = fs::remove_dir_all(&dir);
    ProjectContext::create(&dir, "Pkg", ProjectTemplate::ThreeDStarter).expect("create project");
    fs::create_dir_all(dir.join("assets/tex")).expect("tex dir");
    fs::write(
        dir.join("assets/tex/albedo.png"),
        crate::test_support::MIN_PNG,
    )
    .expect("png");
    let (registry, _) = AssetRegistry::scan(dir.join("assets")).expect("scan");
    let tex = registry.guid_of("tex/albedo.png").expect("tex guid");
    let tex_text = format!("{:032x}", tex.0);
    let scene = dir.join("scenes/main.scene.ron");
    let scene_body = format!(
        r#"(
    format_version: 1,
    name: "main",
    entities: [
        (
            guid: Some("00000000000000000000000000000010"),
            components: [
                Struct(type_name: "MaterialHandle", fields: [("guid", String("{tex_text}")), ("path", String("tex/albedo.png"))]),
            ],
        ),
    ],
)"#
    );
    fs::write(&scene, scene_body).expect("scene");
    let project = ProjectContext::open(&dir).expect("open");
    let service = BuildService;
    let profile = BuildProfile::release_default();
    let pak = dir.join("out.pak");
    let report_path = dir.join("cook_report.json");
    let report = service
        .cook(&project, &profile, &pak, &report_path)
        .expect("cook");
    assert!(pak.is_file());
    assert!(report_path.is_file());
    assert!(!report.assets.is_empty());
    for entry in &report.assets {
        assert!(!entry.guid.is_empty());
        assert!(!entry.path.is_empty());
        assert!(
            matches!(
                entry.reason,
                InclusionReason::DirectFromScene
                    | InclusionReason::ImportedBy { .. }
                    | InclusionReason::ProfileForced
            ),
            "{:?}",
            entry.reason
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn sample_project_validates_clean() {
    let root = temp("validate_ok");
    let _ = std::fs::remove_dir_all(&root);
    ProjectContext::create(&root, "Ok", ProjectTemplate::ThreeDStarter).expect("create");
    let report = validate_project(&root).expect("validate");
    assert!(!report.has_errors(), "{:?}", report.issues);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn broken_sample_reports_missing_asset() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../redox_editor/tests/data/sample_project_broken");
    let report = validate_project(&root).expect("validate");
    assert!(report.has_errors());
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.rule == "MissingAsset" && i.severity == ValidationSeverity::Error),
        "{:?}",
        report.issues
    );
}
