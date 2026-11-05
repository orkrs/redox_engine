//! Browser model on a registry built from a temporary folder (EDT-A6 step 5). No egui here.

use redox_asset::guid::AssetStatus;
use redox_asset::{AssetGuid, AssetRegistry, DependentRef};
use redox_editor_assets::{
    AssetAction, AssetKind, BrowserModel, StatusFilter, actions_for,
};
use std::fs;
use std::path::{Path, PathBuf};

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "redox_editor_browser_{name}_{}",
            std::process::id()
        ));
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

fn write(root: &Path, rel: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create folder");
    }
    fs::write(path, b"data").expect("write file");
}

/// Registry with a failed texture, an imported mesh, a scene and a missing asset.
fn fixture(name: &str) -> (TempRoot, AssetRegistry, AssetGuid, AssetGuid, AssetGuid) {
    let root = TempRoot::new(name);
    write(root.path(), "textures/rock.png");
    write(root.path(), "models/crate.glb");
    write(root.path(), "scenes/main.scene.ron");
    write(root.path(), "models/old.glb");
    let (mut registry, _) = AssetRegistry::scan(root.path()).expect("scan");
    let rock = registry.guid_of("textures/rock.png").expect("rock");
    let crate_guid = registry.guid_of("models/crate.glb").expect("crate");
    let old = registry.guid_of("models/old.glb").expect("old");
    registry.set_status("textures/rock.png", AssetStatus::Failed("bad header".into()));
    registry.set_status("models/crate.glb", AssetStatus::Imported);
    registry.set_status("models/old.glb", AssetStatus::Missing);
    (root, registry, rock, crate_guid, old)
}

fn paths(model: &BrowserModel) -> Vec<String> {
    model.visible().iter().map(|e| e.path.clone()).collect()
}

#[test]
fn status_filter_shows_only_matching_assets() {
    let (_root, registry, _, _, _) = fixture("status_filter");
    let mut model = BrowserModel::new();
    model.refresh(&registry);

    model.filter.status = StatusFilter::Failed;
    assert_eq!(paths(&model), vec!["textures/rock.png".to_string()]);

    model.filter.status = StatusFilter::Missing;
    assert_eq!(paths(&model), vec!["models/old.glb".to_string()]);

    model.filter.status = StatusFilter::Imported;
    assert_eq!(paths(&model), vec!["models/crate.glb".to_string()]);
}

#[test]
fn kind_filter_and_status_filter_combine() {
    let (_root, registry, _, _, _) = fixture("kind_filter");
    let mut model = BrowserModel::new();
    model.refresh(&registry);

    model.filter.kind = Some(AssetKind::Mesh);
    assert_eq!(
        paths(&model),
        vec!["models/crate.glb".to_string(), "models/old.glb".to_string()]
    );

    model.filter.status = StatusFilter::Failed;
    assert!(paths(&model).is_empty());
}

#[test]
fn search_matches_name_and_path_case_insensitively() {
    let (_root, registry, _, _, _) = fixture("search");
    let mut model = BrowserModel::new();
    model.refresh(&registry);

    model.filter.text = "MAIN".into();
    assert_eq!(paths(&model), vec!["scenes/main.scene.ron".to_string()]);

    model.filter.text = "models/".into();
    assert_eq!(paths(&model).len(), 2);
}

#[test]
fn failed_asset_offers_error_and_retry_but_not_spawn() {
    let (_root, registry, rock, _, _) = fixture("actions_failed");
    let mut model = BrowserModel::new();
    model.refresh(&registry);
    let entry = model.entry(rock).expect("rock entry");

    let actions = actions_for(entry);

    assert!(actions.contains(&AssetAction::ShowError));
    assert!(actions.contains(&AssetAction::Reimport));
    assert!(!actions.contains(&AssetAction::Spawn));
}

#[test]
fn missing_asset_offers_only_references_copy_and_delete() {
    let (_root, registry, _, _, old) = fixture("actions_missing");
    let mut model = BrowserModel::new();
    model.refresh(&registry);
    let entry = model.entry(old).expect("old entry");

    assert_eq!(
        actions_for(entry),
        vec![
            AssetAction::ShowReferences,
            AssetAction::CopyGuid,
            AssetAction::Delete
        ]
    );
}

#[test]
fn imported_mesh_offers_spawn_first() {
    let (_root, registry, _, crate_guid, _) = fixture("actions_mesh");
    let mut model = BrowserModel::new();
    model.refresh(&registry);
    let entry = model.entry(crate_guid).expect("crate entry");

    let actions = actions_for(entry);

    assert_eq!(actions.first(), Some(&AssetAction::Spawn));
    assert!(actions.contains(&AssetAction::Rename));
}

#[test]
fn show_references_lists_the_scenes_that_refer_to_the_asset() {
    let (_root, mut registry, _, crate_guid, _) = fixture("references");
    let scene = "scenes/main.scene.ron".to_string();
    registry.set_scene_references(
        &scene,
        vec![DependentRef {
            guid: crate_guid,
            scene: scene.clone(),
            entity: Some("0123456789abcdef0123456789abcdef".into()),
            component: "MeshHandle".into(),
        }],
    );
    let mut model = BrowserModel::new();
    model.refresh(&registry);

    let refs = model.references_of(crate_guid);

    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].scene, "scenes/main.scene.ron");
    assert!(model.references_of(AssetGuid(9)).is_empty());
}

#[test]
fn selection_is_cleared_when_the_asset_disappears() {
    let (_root, registry, rock, _, _) = fixture("selection");
    let mut model = BrowserModel::new();
    model.refresh(&registry);
    model.select(Some(rock));
    assert_eq!(model.selected().map(|e| e.path.as_str()), Some("textures/rock.png"));

    let (empty, empty_registry) = {
        let root = TempRoot::new("selection_empty");
        let (registry, _) = AssetRegistry::scan(root.path()).expect("scan");
        (root, registry)
    };
    model.refresh(&empty_registry);

    assert!(model.selected().is_none());
    drop(empty);
}
