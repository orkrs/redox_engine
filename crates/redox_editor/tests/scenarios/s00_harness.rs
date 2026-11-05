//! Checks of the test handle itself: it must restart on the same files.

use crate::common::EditorHarness;

#[test]
fn a_harness_restarts_on_the_same_files() {
    let mut h = EditorHarness::new();
    let dir = h
        .new_project("restart", "empty")
        .expect("the project is created");
    h.write_file("notes/marker.txt", b"kept on disk");

    let h = h.restart();

    assert!(h.project.is_none(), "restart resets the memory of the editor");
    assert_eq!(h.entity_count(), 0, "the world is empty after restart");
    assert_eq!(
        std::fs::read(dir.join("notes/marker.txt")).expect("the file survives the restart"),
        b"kept on disk"
    );
    assert!(dir.join("project.ron").is_file(), "the project manifest stays on disk");
}
