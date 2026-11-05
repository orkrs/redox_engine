//! TZ §23.16 (part): a damaged `editor_layout.ron` falls back to the default layout and is
//! renamed to `.broken`; autosave settings survive a restart.

use redox_editor_core::autosave::AutosavePolicy;

use crate::common::EditorHarness;

#[test]
fn a_damaged_layout_file_falls_back_to_the_default_and_is_renamed() {
    let h = EditorHarness::new();
    h.write_config("editor_layout.ron", b"(((( damaged layout");

    let outcome = h.load_layout();

    assert!(outcome.used_default, "the default layout is used");
    assert!(outcome.broken_renamed, "the damaged file is renamed");
    assert!(
        h.config_dir().join("editor_layout.ron.broken").exists(),
        "the damaged file is kept as .broken"
    );
    assert!(
        !h.config_dir().join("editor_layout.ron").exists(),
        "the damaged file no longer blocks the next start"
    );
}

#[test]
fn autosave_settings_survive_a_restart() {
    let h = EditorHarness::new();
    let policy = AutosavePolicy {
        interval_secs: 45,
        every_n_transactions: 10,
        keep: 5,
        enabled: true,
    };
    h.save_autosave_policy(policy)
        .expect("the settings are saved");

    let h = h.restart();

    assert_eq!(h.autosave_policy(), policy, "the settings are read back");
}
