//! TZ §23.13: after a crash the editor offers the autosaved state; accepting it restores the
//! autosave as dirty, declining moves the autosaves to `discarded/`, and the original file is
//! never touched by autosave.

use crate::common::EditorHarness;

const MAIN: &str = "scenes/main.scene.ron";
/// Longer than twice the autosave interval, so the session counts as crashed.
const STALE_AFTER_CRASH_SECS: u64 = 300;
/// Longer than the autosave interval of the default policy.
const AUTOSAVE_DUE_SECS: u64 = 200;

/// Opens the sample, edits the main scene, lets autosave run, and crashes the editor.
/// Returns the crashed editor, the project directory, the edited state and the original bytes.
fn crashed_session() -> (
    EditorHarness,
    std::path::PathBuf,
    redox_reflect::WorldSnapshot,
    Vec<u8>,
) {
    let mut h = EditorHarness::new();
    let dir = h.install_sample("sample_project");
    h.open_project(&dir).expect("the sample project opens");
    h.open_start_scene().expect("the main scene opens");
    let original = h.read_file(MAIN);
    let ground = h.find_by_name("Ground").expect("Ground exists");
    h.rename(ground, "Ground edited").expect("Ground is renamed");
    h.advance_clock(AUTOSAVE_DUE_SECS);
    h.tick();
    assert!(!h.autosave_files().is_empty(), "autosave ran after the interval");
    assert_eq!(h.read_file(MAIN), original, "autosave did not touch the original");
    let edited = h.snapshot();
    let crashed = h.crash();
    (crashed, dir, edited, original)
}

#[test]
fn an_unsaved_scene_is_offered_after_a_crash_and_accepting_restores_it() {
    let (mut h, dir, edited, original) = crashed_session();
    h.advance_clock(STALE_AFTER_CRASH_SECS);
    assert!(
        dir.join(".redox/session.lock").exists(),
        "the crash left the session lock"
    );

    h.open_project(&dir).expect("the project opens after the crash");
    assert!(h.recovery_offer().is_some(), "a RecoveryOffer is shown");

    h.accept_recovery().expect("the offer is accepted");
    assert_eq!(h.snapshot(), edited, "the state equals the last autosave");
    assert!(h.is_dirty(), "the recovered state is dirty");
    assert_eq!(h.read_file(MAIN), original, "the original file is still untouched");
}

#[test]
fn declining_the_offer_moves_the_autosaves_to_discarded() {
    let (mut h, dir, _edited, original) = crashed_session();
    h.advance_clock(STALE_AFTER_CRASH_SECS);

    h.open_project(&dir).expect("the project opens after the crash");
    assert!(h.recovery_offer().is_some(), "a RecoveryOffer is shown");

    h.discard_recovery().expect("the offer is declined");
    assert!(h.autosave_files().is_empty(), "autosaves left the working folder");
    assert!(!h.discarded_files().is_empty(), "autosaves are kept in discarded/");
    assert!(h.recovery_offer().is_none());
    assert_eq!(h.read_file(MAIN), original, "the original file is still untouched");
}

#[test]
fn a_clean_close_leaves_no_lock_and_no_autosave() {
    let mut h = EditorHarness::new();
    let dir = h.install_sample("sample_project");
    h.open_project(&dir).expect("the sample project opens");
    h.open_start_scene().expect("the main scene opens");
    let ground = h.find_by_name("Ground").expect("Ground exists");
    h.rename(ground, "Ground edited").expect("Ground is renamed");
    h.advance_clock(AUTOSAVE_DUE_SECS);
    h.tick();
    h.save().expect("the scene is saved before the normal close");

    let mut h = h.restart();
    assert!(!dir.join(".redox/session.lock").exists(), "no lock after a clean close");
    assert!(
        crate::common::list_files(&dir.join(".redox/autosave"), false)
            .iter()
            .all(|p| !p.to_string_lossy().ends_with(".scene.ron")),
        "no autosave after a clean close"
    );

    h.open_project(&dir).expect("the project opens again");
    assert!(h.recovery_offer().is_none(), "a clean close offers nothing");
}
