//! TZ §23.18: entities marked `EditorOnly` never reach the saved scene file, and after a
//! restart they are not there.
//!
//! The spec also names a "snapshot for build". No such API exists yet; the saved scene file is
//! the artifact a build reads, so the check is made on that file.

use crate::common::EditorHarness;

const MAIN: &str = "scenes/main.scene.ron";

/// Number of entities written to a scene file (one `guid:` line per entity).
fn saved_entity_count(text: &str) -> usize {
    text.matches("guid: \"").count()
}

#[test]
fn editor_only_entities_are_not_written_to_the_scene_file() {
    let mut h = EditorHarness::new();
    let dir = h.install_sample("sample_project");
    h.open_project(&dir).expect("the sample project opens");
    h.open_start_scene().expect("the main scene opens");
    let before = saved_entity_count(
        &String::from_utf8(h.read_file(MAIN)).expect("the scene file is text"),
    );

    h.spawn_editor_only();
    h.spawn_editor_only();
    assert_eq!(h.editor_only_count(), 2, "two editor-only entities exist in the editor");
    h.rename(
        h.find_by_name("Ground").expect("Ground exists"),
        "Ground edited",
    )
    .expect("Ground is renamed, so the scene is saved");
    h.save().expect("the scene is saved");

    let text = String::from_utf8(h.read_file(MAIN)).expect("the scene file is text");
    assert_eq!(
        saved_entity_count(&text),
        before,
        "the saved file has the same entities as before the editor-only ones were added"
    );

    let h = h.restart();
    assert_eq!(h.editor_only_count(), 0, "no editor-only entities after restart");
}
