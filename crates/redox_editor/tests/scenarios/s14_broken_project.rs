//! TZ §23.14: a project with a missing asset, an unknown component, a missing plugin and a
//! damaged sidecar opens without a crash and reports each problem. The unknown component is
//! kept in the file on save.

use crate::common::EditorHarness;

#[test]
fn a_broken_project_opens_and_reports_every_problem() {
    let mut h = EditorHarness::new();
    let dir = h.install_sample("sample_project_broken");

    h.open_project(&dir)
        .expect("the broken project opens without an error");
    assert!(
        h.project_warning_tags().iter().any(|t| t == "MissingPlugin"),
        "MissingPlugin is reported: {:?}",
        h.project_warning_tags()
    );

    h.open_start_scene()
        .expect("the start scene opens despite the problems");
    let tags = h.load_warning_tags();
    assert!(tags.iter().any(|t| t == "MissingAsset"), "MissingAsset: {tags:?}");
    assert!(tags.iter().any(|t| t == "UnknownComponent"), "UnknownComponent: {tags:?}");

    let counts = h.asset_counts();
    assert_eq!(counts.damaged, 1, "the damaged sidecar is reported");
    assert!(counts.missing_refs >= 1, "the missing reference is counted");
    assert_eq!(h.entity_count(), 3, "the three entities of the scene are loaded");
}

#[test]
fn an_unknown_component_value_is_written_back_on_save() {
    let mut h = EditorHarness::new();
    let dir = h.install_sample("sample_project_broken");
    h.open_project(&dir).expect("the broken project opens");
    h.open_start_scene().expect("the start scene opens");

    h.save().expect("the scene is saved");

    let text = String::from_utf8(h.read_file("scenes/main.scene.ron"))
        .expect("the scene file is text");
    assert!(
        text.contains("Gizmo3000"),
        "the unknown component value is kept in the saved file"
    );
    assert!(text.contains("speed"), "its fields are kept too");
}
