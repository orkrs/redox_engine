//! TZ §23.8 (part): Play and Stop do not touch the dirty state, do not autosave during Play,
//! and leave the world equal to the snapshot taken before Play.

use crate::common::EditorHarness;

#[test]
fn play_and_stop_keep_dirty_state_and_restore_the_world_exactly() {
    let mut h = EditorHarness::new();
    let dir = h.install_sample("sample_project");
    h.open_project(&dir).expect("the sample project opens");
    h.open_start_scene().expect("the main scene opens");
    let ground = h.find_by_name("Ground").expect("Ground exists");
    h.rename(ground, "Ground edited").expect("Ground is renamed");
    assert!(h.is_dirty(), "the rename makes the scene dirty");
    let before = h.snapshot();

    h.play();
    h.spawn_raw();
    h.advance_clock(600);
    h.tick();
    assert!(h.is_dirty(), "Play keeps the dirty state");
    assert!(
        h.autosave_files().is_empty(),
        "no autosave is written while the game plays"
    );

    h.stop();
    assert!(h.is_dirty(), "Stop keeps the dirty state");
    assert_eq!(h.snapshot(), before, "the world after Stop equals the state before Play");
    assert!(h.find_by_name("Ground edited").is_some(), "GUIDs and names are restored");
}

#[test]
fn autosave_resumes_after_stop() {
    let mut h = EditorHarness::new();
    let dir = h.install_sample("sample_project");
    h.open_project(&dir).expect("the sample project opens");
    h.open_start_scene().expect("the main scene opens");
    let ground = h.find_by_name("Ground").expect("Ground exists");
    h.rename(ground, "Ground edited").expect("Ground is renamed");

    h.play();
    h.advance_clock(600);
    h.tick();
    assert!(h.autosave_files().is_empty(), "nothing is written during Play");
    h.stop();

    h.advance_clock(600);
    h.tick();
    assert!(
        !h.autosave_files().is_empty(),
        "autosave runs again once the game is stopped"
    );
}
