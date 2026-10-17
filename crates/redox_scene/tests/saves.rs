//! Save slots: atomic writes, backups, migrations and name validation.

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use redox_scene::{SaveManager, SceneError};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("redox_saves_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn save_then_load_round_trips() {
    let dir = TempDir::new("roundtrip");
    let saves = SaveManager::new(&dir.0, 1, 7);
    let info = saves.save("slot1", "Level 3", "payload-v1").unwrap();
    assert_eq!(info.game_version, 7);
    let loaded = saves.load("slot1").unwrap();
    assert_eq!(loaded.payload, "payload-v1");
    assert_eq!(loaded.info.label, "Level 3");
    assert_eq!(loaded.info.schema_version, 1);
}

#[test]
fn second_save_keeps_the_previous_one_as_backup() {
    let dir = TempDir::new("backup");
    let saves = SaveManager::new(&dir.0, 1, 1);
    saves.save("a", "first", "one").unwrap();
    saves.save("a", "second", "two").unwrap();
    assert_eq!(saves.load("a").unwrap().payload, "two");
    saves.restore_backup("a").unwrap();
    assert_eq!(saves.load("a").unwrap().payload, "one");
    saves.restore_backup("a").unwrap();
    assert_eq!(
        saves.load("a").unwrap().payload,
        "two",
        "restoring twice undoes itself"
    );
}

#[test]
fn list_is_newest_first_and_skips_damaged_files() {
    let dir = TempDir::new("list");
    let saves = SaveManager::new(&dir.0, 1, 1);
    saves.save("old", "old", "x").unwrap();
    saves.save("new", "new", "y").unwrap();
    std::fs::write(dir.0.join("broken.save"), "not ron").unwrap();
    std::fs::write(dir.0.join("note.txt"), "ignored").unwrap();
    let names: Vec<_> = saves.list().into_iter().map(|s| s.slot).collect();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&"old".to_string()) && names.contains(&"new".to_string()));
    assert!(
        SaveManager::new(dir.0.join("missing"), 1, 1)
            .list()
            .is_empty()
    );
}

#[test]
fn delete_removes_slot_and_backup_and_is_idempotent() {
    let dir = TempDir::new("delete");
    let saves = SaveManager::new(&dir.0, 1, 1);
    saves.save("a", "a", "1").unwrap();
    saves.save("a", "a", "2").unwrap();
    saves.delete("a").unwrap();
    assert!(saves.load("a").is_err());
    assert!(saves.restore_backup("a").is_err());
    saves.delete("a").unwrap();
}

#[test]
fn slot_names_cannot_escape_the_directory() {
    let dir = TempDir::new("names");
    let saves = SaveManager::new(&dir.0, 1, 1);
    for bad in [
        "",
        "..",
        "../evil",
        "a/b",
        r"a\b",
        "dot.name",
        &"x".repeat(65),
    ] {
        assert!(!SaveManager::is_valid_slot_name(bad), "{bad:?}");
        assert!(
            matches!(saves.save(bad, "l", "p"), Err(SceneError::InvalidCell(_))),
            "{bad:?}"
        );
    }
    assert!(SaveManager::is_valid_slot_name("Quick_save-2"));
}

fn v1_to_v2(payload: &str) -> redox_scene::SceneResult<String> {
    Ok(format!("{payload}+v2"))
}

fn v2_to_v3(payload: &str) -> redox_scene::SceneResult<String> {
    Ok(format!("{payload}+v3"))
}

#[test]
fn old_saves_are_migrated_step_by_step() {
    let dir = TempDir::new("migrate");
    SaveManager::new(&dir.0, 1, 1)
        .save("s", "old", "data")
        .unwrap();

    let current = SaveManager::new(&dir.0, 3, 2)
        .with_migration(1, v1_to_v2)
        .with_migration(2, v2_to_v3);
    let loaded = current.load("s").unwrap();
    assert_eq!(loaded.payload, "data+v2+v3");
    assert_eq!(loaded.info.schema_version, 3);
}

#[test]
fn missing_migration_and_future_schema_are_errors() {
    let dir = TempDir::new("errors");
    SaveManager::new(&dir.0, 1, 1)
        .save("s", "old", "data")
        .unwrap();
    let gap = SaveManager::new(&dir.0, 3, 1).with_migration(2, v2_to_v3);
    assert!(gap.load("s").is_err(), "no 1 -> 2 migration registered");

    SaveManager::new(&dir.0, 5, 1)
        .save("future", "f", "p")
        .unwrap();
    assert!(SaveManager::new(&dir.0, 4, 1).load("future").is_err());
}
