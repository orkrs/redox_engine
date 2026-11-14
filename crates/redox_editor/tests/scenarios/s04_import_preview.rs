//! TZ §23.4: import preview settings roundtrip (EDT-B4 step 5).

use crate::common::{EditorHarness, make_png};
use redox_editor_assets::{ImportPresetId, settings_for_preset};

const TEXTURE: &str = "assets/textures/preview.png";

#[test]
fn import_preview_settings_roundtrip() {
    let mut h = EditorHarness::new();
    let dir = h.install_sample("sample_project");
    h.open_project(&dir).expect("open");
    h.write_file(TEXTURE, &make_png(32, 32));
    let service = h.assets_mut().expect("assets");
    let preset = settings_for_preset(ImportPresetId::TextureNormal);
    service
        .set_per_asset_settings(TEXTURE, &preset)
        .expect("settings");
    h.request_import(TEXTURE);
    h.wait_imports_idle();
    let read = service.per_asset_settings(TEXTURE).expect("read back");
    assert_eq!(read.preset, ImportPresetId::TextureNormal);
    assert!(read.generate_mipmaps);
}