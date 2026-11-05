//! Incremental registry updates driven by file-system events (EDT-A6 step 2).

use redox_asset::guid::AssetStatus;
use redox_asset::{AssetEvent, AssetGuid, AssetRegistry, DependentRef, RegistryChange};
use std::fs;
use std::path::{Path, PathBuf};

/// A temporary asset root removed when the test ends.
struct TempRoot(PathBuf);

impl TempRoot {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "redox_asset_events_{name}_{}",
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

/// Writes a file below `root`, creating folders as needed, and returns its absolute path.
fn write(root: &Path, rel: &str, bytes: &[u8]) -> PathBuf {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create folder");
    }
    fs::write(&path, bytes).expect("write file");
    path
}

#[test]
fn created_file_gets_sidecar_and_guid() {
    let root = TempRoot::new("created");
    let (mut reg, _) = AssetRegistry::scan(root.path()).expect("scan");
    let file = write(root.path(), "textures/a.png", b"pixels");

    let change = reg.apply_event(&AssetEvent::Created(file));

    let RegistryChange::Added { path, guid } = change else {
        panic!("expected Added");
    };
    assert_eq!(path, "textures/a.png");
    assert!(root.path().join("textures/a.png.meta").exists());
    assert_eq!(reg.guid_of("textures/a.png"), Some(guid));
    assert_eq!(reg.status("textures/a.png"), Some(&AssetStatus::Outdated));
}

#[test]
fn external_rename_with_sidecar_keeps_guid() {
    let root = TempRoot::new("rename");
    let file = write(root.path(), "a.png", b"x");
    let (mut reg, _) = AssetRegistry::scan(root.path()).expect("scan");
    let guid = reg.guid_of("a.png").expect("guid");

    fs::create_dir_all(root.path().join("moved")).expect("folder");
    let target = root.path().join("moved/b.png");
    fs::rename(&file, &target).expect("move asset");
    fs::rename(
        root.path().join("a.png.meta"),
        root.path().join("moved/b.png.meta"),
    )
    .expect("move sidecar");

    let change = reg.apply_event(&AssetEvent::Renamed {
        from: file,
        to: target,
    });

    assert_eq!(
        change,
        RegistryChange::Moved {
            from: "a.png".into(),
            to: "moved/b.png".into(),
            guid,
        }
    );
    assert_eq!(reg.path_of(guid), Some("moved/b.png"));
}

#[test]
fn rename_seen_as_remove_then_create_keeps_guid() {
    let root = TempRoot::new("pair");
    let file = write(root.path(), "a.png", b"x");
    let (mut reg, _) = AssetRegistry::scan(root.path()).expect("scan");
    let guid = reg.guid_of("a.png").expect("guid");

    fs::create_dir_all(root.path().join("moved")).expect("folder");
    let target = root.path().join("moved/b.png");
    fs::rename(&file, &target).expect("move asset");
    fs::rename(
        root.path().join("a.png.meta"),
        root.path().join("moved/b.png.meta"),
    )
    .expect("move sidecar");

    let removed = reg.apply_event(&AssetEvent::Removed(file));
    assert!(matches!(removed, RegistryChange::Missing { .. }));
    let created = reg.apply_event(&AssetEvent::Created(target));

    assert_eq!(
        created,
        RegistryChange::Moved {
            from: "a.png".into(),
            to: "moved/b.png".into(),
            guid,
        }
    );
    assert_eq!(reg.path_of(guid), Some("moved/b.png"));
}

#[test]
fn removed_file_becomes_missing_and_keeps_guid() {
    let root = TempRoot::new("removed");
    let file = write(root.path(), "a.png", b"x");
    let (mut reg, _) = AssetRegistry::scan(root.path()).expect("scan");
    let guid = reg.guid_of("a.png").expect("guid");

    fs::remove_file(&file).expect("remove asset");
    fs::remove_file(root.path().join("a.png.meta")).expect("remove sidecar");
    let change = reg.apply_event(&AssetEvent::Removed(file));

    assert_eq!(
        change,
        RegistryChange::Missing {
            path: "a.png".into(),
            guid,
        }
    );
    assert_eq!(reg.status("a.png"), Some(&AssetStatus::Missing));
    assert_eq!(reg.path_of(guid), Some("a.png"));
}

#[test]
fn changed_content_becomes_outdated_and_same_content_stays_unchanged() {
    let root = TempRoot::new("modified");
    let file = write(root.path(), "a.png", b"first");
    let (reg, _) = AssetRegistry::scan(root.path()).expect("scan");
    let guid = reg.guid_of("a.png").expect("guid");
    reg.mark_imported("a.png", 1).expect("mark imported");
    let (mut reg, _) = AssetRegistry::scan(root.path()).expect("rescan");
    assert_eq!(reg.status("a.png"), Some(&AssetStatus::Imported));

    let same = reg.apply_event(&AssetEvent::Modified(file.clone()));
    assert_eq!(
        same,
        RegistryChange::Unchanged {
            path: "a.png".into(),
            guid,
        }
    );

    fs::write(&file, b"second").expect("change content");
    let changed = reg.apply_event(&AssetEvent::Modified(file));
    assert_eq!(
        changed,
        RegistryChange::Outdated {
            path: "a.png".into(),
            guid,
        }
    );
    assert_eq!(reg.status("a.png"), Some(&AssetStatus::Outdated));
}

#[test]
fn temporary_and_backup_files_are_ignored() {
    let root = TempRoot::new("ignored");
    let (mut reg, _) = AssetRegistry::scan(root.path()).expect("scan");

    for name in ["a.png.tmp.1234", "a.png.bak", "b.png.meta"] {
        let file = write(root.path(), name, b"junk");
        let change = reg.apply_event(&AssetEvent::Created(file));
        assert_eq!(change, RegistryChange::Ignored, "{name} must be ignored");
        assert!(
            !root.path().join(format!("{name}.meta")).exists(),
            "{name} must not get a sidecar"
        );
    }
    assert!(reg.is_empty());
}

#[test]
fn dependents_find_scenes_that_reference_a_guid() {
    let root = TempRoot::new("dependents");
    let file = write(root.path(), "a.png", b"x");
    let (mut reg, _) = AssetRegistry::scan(root.path()).expect("scan");
    let guid = reg.guid_of("a.png").expect("guid");
    let _ = reg.apply_event(&AssetEvent::Modified(file));

    let scene = "scenes/main.scene.ron".to_string();
    reg.set_scene_references(
        &scene,
        vec![DependentRef {
            guid,
            scene: scene.clone(),
            entity: Some("0123456789abcdef0123456789abcdef".into()),
            component: "MaterialHandle".into(),
        }],
    );

    let found = reg.dependents(guid);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].scene, "scenes/main.scene.ron");
    assert!(reg.dependents(AssetGuid(1)).is_empty());

    reg.remove_scene_references(&scene);
    assert!(reg.dependents(guid).is_empty());
}

#[test]
fn copy_with_sidecar_gets_a_fresh_guid() {
    let root = TempRoot::new("copy");
    write(root.path(), "a.png", b"x");
    let (mut reg, _) = AssetRegistry::scan(root.path()).expect("scan");
    let original = reg.guid_of("a.png").expect("guid");

    fs::copy(root.path().join("a.png"), root.path().join("b.png")).expect("copy asset");
    fs::copy(
        root.path().join("a.png.meta"),
        root.path().join("b.png.meta"),
    )
    .expect("copy sidecar");
    let change = reg.apply_event(&AssetEvent::Created(root.path().join("b.png")));

    let RegistryChange::Added { guid, .. } = change else {
        panic!("expected Added for the copy");
    };
    assert_ne!(guid, original);
    assert_eq!(reg.path_of(original), Some("a.png"));
    assert_eq!(reg.path_of(guid), Some("b.png"));
}

#[test]
fn renamed_folder_keeps_guids_of_its_assets() {
    let root = TempRoot::new("folder");
    write(root.path(), "dir/a.png", b"x");
    let (mut reg, _) = AssetRegistry::scan(root.path()).expect("scan");
    let guid = reg.guid_of("dir/a.png").expect("guid");

    fs::rename(root.path().join("dir"), root.path().join("dir2")).expect("rename folder");
    let change = reg.apply_event(&AssetEvent::Renamed {
        from: root.path().join("dir"),
        to: root.path().join("dir2"),
    });

    assert_eq!(
        change,
        RegistryChange::Subtree {
            path: "dir2".into()
        }
    );
    assert_eq!(reg.path_of(guid), Some("dir2/a.png"));
    assert_eq!(reg.guid_of("dir/a.png"), None);
}
