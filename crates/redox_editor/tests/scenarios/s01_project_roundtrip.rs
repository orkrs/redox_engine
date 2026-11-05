//! TZ §23.1: a project from a template survives a restart with the same start scene.

use crate::common::EditorHarness;

#[test]
fn a_project_from_the_starter_template_survives_a_restart() {
    let mut h = EditorHarness::new();
    let dir = h
        .new_project("starter", "3d_starter")
        .expect("the project is created from the template");
    assert_eq!(h.start_scene().as_deref(), Some("scenes/main.scene.ron"));
    h.open_start_scene().expect("the start scene opens");
    assert!(
        h.entity_count() >= 3,
        "the starter scene has a camera, a light and a cube"
    );
    let before = h.snapshot();

    let mut h = h.restart();
    h.open_project(&dir).expect("the project opens again");
    h.open_start_scene().expect("the start scene opens again");

    assert_eq!(h.snapshot(), before, "the start scene is the same after restart");
    assert!(
        h.recent_projects().contains(&dir),
        "the project is in the recent list"
    );
    assert!(h.project_warning_tags().is_empty());
}
