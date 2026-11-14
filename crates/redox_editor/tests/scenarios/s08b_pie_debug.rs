//! TZ §23.8 extensions: step frames, save guard during PIE, simulate mode.

use crate::common::EditorHarness;
use redox_editor_debugger::PieRunMode;

#[test]
fn step_advances_frame_counter_without_touching_dirty_baseline() {
    let mut h = EditorHarness::new();
    let dir = h.install_sample("sample_project");
    h.open_project(&dir).expect("open project");
    h.open_start_scene().expect("open scene");
    assert!(!h.is_dirty());

    h.play();
    let mut steps = 0u64;
    for _ in 0..4 {
        let mut world = h.ctx.world.lock();
        let registry = h.ctx.registry.lock();
        h.pie.step(&mut world, &registry, &mut |_, _| {});
        steps = h.pie.frame_count;
    }
    assert_eq!(steps, 4);
    assert!(h.is_dirty());
    h.stop();
    assert!(!h.pie.running);
}

#[test]
fn save_while_playing_is_rejected() {
    let mut h = EditorHarness::new();
    let dir = h.install_sample("sample_project");
    h.open_project(&dir).expect("open project");
    h.open_start_scene().expect("open scene");
    h.play();
    assert!(h.save().is_err(), "save must fail during PIE");
    h.stop();
    h.save().expect("save works after stop");
}

#[test]
fn simulate_mode_starts_without_gameplay_input() {
    let mut h = EditorHarness::new();
    let dir = h.install_sample("sample_project");
    h.open_project(&dir).expect("open project");
    h.open_start_scene().expect("open scene");
    {
        let world = h.ctx.world.lock();
        let registry = h.ctx.registry.lock();
        h.pie
            .start_with_mode(&world, &registry, PieRunMode::Simulate);
    }
    assert!(h.pie.running);
    assert!(!h.pie.gameplay_input_enabled());
    h.stop();
}