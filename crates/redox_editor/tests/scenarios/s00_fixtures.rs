//! The committed sample projects must open, and the broken one must open with a report.

use crate::common::{CHECKER_GUID, EditorHarness};

#[test]
fn the_sample_project_opens_without_warnings() {
    let mut h = EditorHarness::new();
    let dir = h.install_sample("sample_project");

    h.open_project(&dir).expect("the sample project opens");
    assert!(
        h.project_warning_tags().is_empty(),
        "project warnings: {:?}",
        h.project_warning_tags()
    );

    h.open_start_scene().expect("the start scene opens");
    assert!(
        h.load_warning_tags().is_empty(),
        "scene warnings: {:?}",
        h.load_warning_tags()
    );
    assert_eq!(h.entity_count(), 7, "camera, sun, five objects");
    assert_eq!(h.asset_counts().missing_refs, 0);
    assert_eq!(h.asset_counts().damaged, 0);
    assert!(h.find_by_name("Crate Top").is_some());
}

#[test]
fn the_sample_asset_reference_keeps_its_guid() {
    let mut h = EditorHarness::new();
    let dir = h.install_sample("sample_project");
    h.open_project(&dir).expect("the sample project opens");
    h.open_start_scene().expect("the start scene opens");

    let ground = h.find_by_name("Ground").expect("the sample has Ground");
    let (guid, path) = h
        .asset_ref(ground, "MaterialHandle")
        .expect("Ground references the checker texture");

    assert_eq!(guid.as_deref(), Some(format!("{CHECKER_GUID:032x}").as_str()));
    assert_eq!(path.as_deref(), Some("assets/textures/checker.png"));
}
