//! EDT-C2 audit tests.

use redox_editor_assets::AssetService;
use redox_editor_assets::{
    AuditConfig, ReferenceViewerModel, build_size_map, export_size_map_csv, find_duplicates,
    find_orphans, parse_size_map_csv,
};
use redox_reflect::ReflectValue;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("redox_audit_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp");
        Self(dir)
    }
}

fn write(root: &Path, rel: &str, bytes: &[u8]) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("dir");
    }
    fs::write(path, bytes).expect("write");
}

#[derive(Serialize)]
struct TestScene {
    format_version: u32,
    name: String,
    entities: Vec<TestEntity>,
}

#[derive(Serialize)]
struct TestEntity {
    guid: Option<String>,
    components: Vec<ReflectValue>,
    unknown_components: Vec<ReflectValue>,
}

const SCENE: &str = "scenes/main.scene.ron";

fn scene_with_ref(guid: &str, path: &str) -> String {
    let mesh =
        ReflectValue::asset_ref("MeshHandle", Some(guid.to_string()), Some(path.to_string()));
    let scene = TestScene {
        format_version: 1,
        name: "main".into(),
        entities: vec![TestEntity {
            guid: Some("0123456789abcdef0123456789abcdef".into()),
            components: vec![mesh],
            unknown_components: Vec::new(),
        }],
    };
    ron::ser::to_string_pretty(&scene, ron::ser::PrettyConfig::new()).expect("scene")
}

#[test]
fn orphan_png_not_referenced_from_scenes() {
    let root = TempRoot::new("orphan");
    write(&root.0, "used.png", b"x");
    write(&root.0, "extra.png", b"y");
    let service = AssetService::open(&root.0).expect("open");
    let guid = service.guid_of("used.png").expect("guid");
    write(
        &root.0,
        SCENE,
        scene_with_ref(&guid.to_string(), "used.png").as_bytes(),
    );
    let mut service = AssetService::open(&root.0).expect("reopen");
    service.rescan().expect("rescan");
    let orphans = find_orphans(service.registry(), &AuditConfig::default());
    assert!(orphans.contains(&"extra.png".to_string()));
    assert!(!orphans.contains(&"used.png".to_string()));
}

#[test]
fn duplicate_content_hash_groups() {
    let root = TempRoot::new("dup");
    let bytes = b"same-bytes";
    write(&root.0, "a.png", bytes);
    write(&root.0, "b/a.png", bytes);
    let service = AssetService::open(&root.0).expect("open");
    let groups = find_duplicates(service.registry());
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].members.len(), 2);
}

#[test]
fn size_map_children_sum_parent() {
    let root = TempRoot::new("size");
    write(&root.0, "textures/a.png", &[0u8; 10]);
    write(&root.0, "textures/b.png", &[0u8; 20]);
    let service = AssetService::open(&root.0).expect("open");
    let tree = build_size_map(service.registry());
    let textures = tree
        .children
        .iter()
        .find(|c| c.path == "textures")
        .expect("textures");
    assert_eq!(textures.disk_bytes, 30);
    assert_eq!(tree.disk_bytes, 30);
}

#[test]
fn export_csv_roundtrip() {
    let root = TempRoot::new("csv");
    write(&root.0, "a.png", &[1u8; 5]);
    let service = AssetService::open(&root.0).expect("open");
    let tree = build_size_map(service.registry());
    let csv = export_size_map_csv(&tree);
    let rows = parse_size_map_csv(&csv).expect("parse");
    assert!(rows.iter().any(|r| r.disk_bytes == 5));
}

#[test]
fn reference_viewer_lists_incoming() {
    let root = TempRoot::new("refs");
    write(&root.0, "tex.png", b"x");
    let service = AssetService::open(&root.0).expect("open");
    let guid = service.guid_of("tex.png").expect("guid");
    write(
        &root.0,
        SCENE,
        scene_with_ref(&guid.to_string(), "tex.png").as_bytes(),
    );
    let service = AssetService::open(&root.0).expect("reopen");
    let mut model = ReferenceViewerModel::new();
    model.select(guid, service.registry(), service.references_index());
    assert_eq!(model.view().incoming.len(), 1);
}
