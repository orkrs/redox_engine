//! Viewport settings persistence (EditorSettings).

use redox_editor_core::settings::EditorSettings;
use redox_editor_core::viewport_prefs::ViewportRuntimeSettings;

#[test]
fn viewport_settings_roundtrip_ron() {
    let settings = EditorSettings {
        viewport: ViewportRuntimeSettings {
            procedural_sky: true,
            gi_enabled: true,
            fog_enabled: false,
            sdf_debug: true,
            show_stats_overlay: true,
            vg_enabled: false,
        },
        ..Default::default()
    };
    let text = ron::ser::to_string(&settings).expect("serialize");
    let back: EditorSettings = ron::from_str(&text).expect("deserialize");
    assert_eq!(back.viewport, settings.viewport);
}
