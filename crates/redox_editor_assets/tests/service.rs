//! Asset service on a temporary assets root (EDT-A6 steps 3 and 6).

use redox_asset::AssetGuid;
use redox_editor_assets::{
    AssetChange, AssetMoveOp, AssetService, DeleteOutcome, RefFix, RefResolution,
};
use redox_reflect::ReflectValue;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const ENTITY: &str = "0123456789abcdef0123456789abcdef";
const MISSING_GUID: &str = "ffffffffffffffffffffffffffffffff";

/// A temporary assets root removed when the test ends.
struct TempRoot(PathBuf);

impl TempRoot {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("redox_editor_assets_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp root");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write(root: &Path, rel: &str, bytes: &[u8]) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create folder");
    }
    fs::write(path, bytes).expect("write file");
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

/// A scene with one entity that holds one `MeshHandle` reference.
fn scene_with_ref(guid: Option<&str>, path: Option<&str>) -> String {
    let mesh = ReflectValue::asset_ref(
        "MeshHandle",
        guid.map(str::to_owned),
        path.map(str::to_owned),
    );
    let scene = TestScene {
        format_version: 1,
        name: "main".into(),
        entities: vec![TestEntity {
            guid: Some(ENTITY.into()),
            components: vec![mesh],
            unknown_components: Vec::new(),
        }],
    };
    ron::ser::to_string_pretty(&scene, ron::ser::PrettyConfig::new()).expect("serialize scene")
}

const SCENE: &str = "scenes/main.scene.ron";

#[test]
fn closed_scene_keeps_resolving_after_a_move() {
    let root = TempRoot::new("move_keeps_link");
    write(root.path(), "a.png", b"x");
    let mut service = AssetService::open(root.path()).expect("open");
    let guid = service.guid_of("a.png").expect("guid");
    write(
        root.path(),
        SCENE,
        scene_with_ref(Some(&guid.to_string()), Some("a.png")).as_bytes(),
    );
    service.rescan().expect("rescan indexes the scene");

    let report = service.move_asset("a.png", "moved/b.png").expect("move");
    assert_eq!(report.scenes, vec![SCENE.to_string()]);
    let rewritten = service.update_path_hints(guid).expect("hints");
    assert_eq!(rewritten, 1);

    let reopened = AssetService::open(root.path()).expect("reopen");
    assert_eq!(reopened.resolve(guid), Some("moved/b.png"));
    let text = fs::read_to_string(root.path().join(SCENE)).expect("read scene");
    assert!(text.contains("moved/b.png"));
    assert!(reopened.missing_refs().is_empty());
}

#[test]
fn path_only_reference_restores_the_guid() {
    let root = TempRoot::new("legacy_ref");
    write(root.path(), "a.png", b"x");
    write(
        root.path(),
        SCENE,
        scene_with_ref(None, Some("a.png")).as_bytes(),
    );
    let service = AssetService::open(root.path()).expect("open");
    let guid = service.guid_of("a.png").expect("guid");

    match service.resolve_ref(None, Some("a.png")) {
        RefResolution::Restored {
            guid: restored,
            path,
        } => {
            assert_eq!(restored, guid);
            assert_eq!(path, "a.png");
        }
        other => panic!("expected Restored, got {other:?}"),
    }
    assert!(service.missing_refs().is_empty());
}

#[test]
fn delete_without_force_lists_dependents_and_keeps_the_file() {
    let root = TempRoot::new("delete_blocked");
    write(root.path(), "a.png", b"x");
    let mut service = AssetService::open(root.path()).expect("open");
    let guid = service.guid_of("a.png").expect("guid");
    write(
        root.path(),
        SCENE,
        scene_with_ref(Some(&guid.to_string()), Some("a.png")).as_bytes(),
    );
    service.rescan().expect("rescan");

    let outcome = service
        .delete_asset("a.png", false)
        .expect("delete request");

    match outcome {
        DeleteOutcome::Blocked { dependents } => {
            assert_eq!(dependents.len(), 1);
            assert_eq!(dependents[0].scene, SCENE);
            assert_eq!(dependents[0].entity.as_deref(), Some(ENTITY));
        }
        other => panic!("expected Blocked, got {other:?}"),
    }
    assert!(root.path().join("a.png").exists());
}

#[test]
fn delete_with_force_leaves_references_missing_not_removed() {
    let root = TempRoot::new("delete_forced");
    write(root.path(), "a.png", b"x");
    let mut service = AssetService::open(root.path()).expect("open");
    let guid = service.guid_of("a.png").expect("guid");
    write(
        root.path(),
        SCENE,
        scene_with_ref(Some(&guid.to_string()), Some("a.png")).as_bytes(),
    );
    service.rescan().expect("rescan");

    let outcome = service.delete_asset("a.png", true).expect("forced delete");
    assert!(matches!(outcome, DeleteOutcome::Deleted { .. }));
    assert!(!root.path().join("a.png").exists());

    let missing = service.missing_refs();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].guid.as_deref(), Some(guid.to_string().as_str()));
    let text = fs::read_to_string(root.path().join(SCENE)).expect("read scene");
    assert!(
        text.contains(&guid.to_string()),
        "reference must stay in the scene"
    );
}

#[test]
fn external_rename_is_seen_by_tick() {
    let root = TempRoot::new("external_rename");
    write(root.path(), "a.png", b"x");
    let mut service = AssetService::open(root.path()).expect("open");
    let guid = service.guid_of("a.png").expect("guid");

    fs::create_dir_all(root.path().join("moved")).expect("folder");
    fs::rename(root.path().join("a.png"), root.path().join("moved/b.png")).expect("rename");
    fs::rename(
        root.path().join("a.png.meta"),
        root.path().join("moved/b.png.meta"),
    )
    .expect("rename sidecar");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut seen: Vec<AssetChange> = Vec::new();
    while Instant::now() < deadline && !seen.iter().any(|c| matches!(c, AssetChange::Moved { .. }))
    {
        seen.extend(service.tick());
        std::thread::sleep(Duration::from_millis(50));
    }

    assert!(seen.contains(&AssetChange::Moved {
        from: "a.png".into(),
        to: "moved/b.png".into(),
        guid,
    }));
    assert_eq!(service.resolve(guid), Some("moved/b.png"));
}

#[test]
fn undo_of_a_move_restores_the_path_and_the_guid() {
    let root = TempRoot::new("undo_move");
    write(root.path(), "a.png", b"x");
    let mut service = AssetService::open(root.path()).expect("open");
    let guid = service.guid_of("a.png").expect("guid");
    let op = AssetMoveOp {
        from: "a.png".into(),
        to: "b.png".into(),
    };

    op.apply(&mut service).expect("apply");
    assert_eq!(service.guid_of("b.png"), Some(guid));

    op.revert(&mut service).expect("revert");
    assert_eq!(service.guid_of("a.png"), Some(guid));
    assert!(root.path().join("a.png").exists());
    assert!(!root.path().join("b.png").exists());
}

#[test]
fn damaged_sidecar_is_reported_and_can_be_repaired() {
    let root = TempRoot::new("damaged");
    write(root.path(), "c.png", b"x");
    write(root.path(), "c.png.meta", b"not ron {{");
    let mut service = AssetService::open(root.path()).expect("open must not fail");

    assert!(service.report().damaged.contains(&PathBuf::from("c.png")));
    assert_eq!(service.guid_of("c.png"), None);

    service.repair_sidecar("c.png").expect("repair");
    assert!(service.guid_of("c.png").is_some());
}

#[test]
fn copy_with_sidecar_gets_a_new_guid_and_references_stay_on_the_original() {
    let root = TempRoot::new("copy_on_open");
    write(root.path(), "a.png", b"x");
    let original = {
        let service = AssetService::open(root.path()).expect("first open");
        service.guid_of("a.png").expect("guid")
    };
    write(
        root.path(),
        SCENE,
        scene_with_ref(Some(&original.to_string()), Some("a.png")).as_bytes(),
    );
    fs::copy(root.path().join("a.png"), root.path().join("copy.png")).expect("copy asset");
    fs::copy(
        root.path().join("a.png.meta"),
        root.path().join("copy.png.meta"),
    )
    .expect("copy sidecar");

    let service = AssetService::open(root.path()).expect("reopen");

    assert!(
        service
            .report()
            .regenerated
            .contains(&PathBuf::from("copy.png"))
    );
    let copy_guid = service.guid_of("copy.png").expect("copy guid");
    assert_ne!(copy_guid, original);
    assert_eq!(service.resolve(original), Some("a.png"));
}

#[test]
fn reference_to_an_absent_asset_is_reported_on_open() {
    let root = TempRoot::new("missing_on_open");
    write(
        root.path(),
        SCENE,
        scene_with_ref(Some(MISSING_GUID), Some("gone.png")).as_bytes(),
    );

    let service = AssetService::open(root.path()).expect("open must not fail");

    let report = service.report();
    assert_eq!(report.missing_refs.len(), 1);
    assert_eq!(report.missing_refs[0].scene, SCENE);
    assert_eq!(report.missing_refs[0].path.as_deref(), Some("gone.png"));
    assert!(report.unreadable_scenes.is_empty());
}

#[test]
fn fix_reassigns_a_missing_reference_to_an_existing_asset() {
    let root = TempRoot::new("fix_reassign");
    write(root.path(), "real.png", b"x");
    write(
        root.path(),
        SCENE,
        scene_with_ref(Some(MISSING_GUID), Some("gone.png")).as_bytes(),
    );
    let mut service = AssetService::open(root.path()).expect("open");
    let target = service.missing_refs().remove(0);
    let real: AssetGuid = service.guid_of("real.png").expect("guid");

    let changed = service
        .fix_missing_ref(&target, &RefFix::Reassign(real))
        .expect("fix");

    assert_eq!(changed, 1);
    assert!(service.missing_refs().is_empty());
}
