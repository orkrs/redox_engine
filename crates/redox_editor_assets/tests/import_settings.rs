//! EDT-B4 step 1: import settings and presets.

use redox_asset::guid::AssetStatus;
use redox_editor_assets::{AssetService, ImportPresetId, settings_for_preset};
use std::fs;
use std::path::{Path, PathBuf};

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("redox_import_settings_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp");
        Self(dir)
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
        fs::create_dir_all(parent).expect("dir");
    }
    fs::write(path, bytes).expect("write");
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend_from_slice(&13u32.to_be_bytes());
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    bytes
}

#[test]
fn reimport_after_touch_keeps_settings() {
    let root = TempRoot::new("touch");
    write(&root.0, "a.png", &png(4, 4));
    let mut service = AssetService::open(&root.0).expect("open");
    let preset = settings_for_preset(ImportPresetId::TextureColor);
    service
        .set_per_asset_settings("a.png", &preset)
        .expect("write settings");
    service.import("a.png");
    wait_idle(&mut service);
    let before = service.per_asset_settings("a.png").expect("read");
    write(&root.0, "a.png", &png(4, 5));
    service.tick();
    service.import("a.png");
    wait_idle(&mut service);
    let after = service.per_asset_settings("a.png").expect("read");
    assert_eq!(before.preset, after.preset);
    assert_eq!(before.scale, after.scale);
}

#[test]
fn preset_change_marks_outdated() {
    let root = TempRoot::new("preset");
    write(&root.0, "a.png", &png(8, 8));
    let mut service = AssetService::open(&root.0).expect("open");
    service.import("a.png");
    wait_idle(&mut service);
    assert_eq!(
        service.status("a.png").map(|s| s.clone()),
        Some(AssetStatus::Imported)
    );
    service
        .apply_import_preset("a.png", ImportPresetId::TextureNormal)
        .expect("preset");
    assert_eq!(
        service.status("a.png").map(|s| s.clone()),
        Some(AssetStatus::Outdated)
    );
}

fn wait_idle(service: &mut AssetService) {
    for _ in 0..1000 {
        service.tick();
        if service.import_counts().importing == 0 {
            break;
        }
    }
}