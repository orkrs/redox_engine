//! TZ §23.3: move, rename, duplicate, delete and reparent are undone and redone step by step;
//! the world, GUIDs and hierarchy come back; dirty follows the save point.

use crate::common::EditorHarness;

#[test]
fn every_edit_is_undone_and_redone_with_identity_and_dirty_state_intact() {
    let mut h = EditorHarness::new();
    h.new_project("edit_undo", "empty")
        .expect("the project is created");
    h.new_scene("scenes/edit.scene.ron")
        .expect("the scene is created");
    let alpha = h.create_entity("Alpha").expect("Alpha is created");
    let beta = h.create_entity("Beta").expect("Beta is created");
    h.save().expect("the scene is saved");
    assert!(!h.is_dirty(), "a saved scene is clean");

    // history[0] is the save point; history[i] is the state after operation i.
    let mut history = vec![h.snapshot()];

    h.move_to(alpha, [1.0, 0.0, 0.0]).expect("Alpha moves");
    history.push(h.snapshot());
    h.rename(alpha, "Alpha renamed").expect("Alpha is renamed");
    history.push(h.snapshot());
    let copy = h.duplicate(alpha).expect("Alpha is duplicated");
    history.push(h.snapshot());
    h.reparent(beta, Some(alpha)).expect("Beta gets a parent");
    history.push(h.snapshot());
    h.delete(copy).expect("the copy is deleted");
    history.push(h.snapshot());
    assert!(h.is_dirty(), "edits make the scene dirty");

    let operations = history.len() - 1;
    for step in (0..operations).rev() {
        assert!(h.undo(), "undo step {step} is available");
        assert_eq!(h.snapshot(), history[step], "undo back to state {step}");
    }
    assert!(
        !h.is_dirty(),
        "back at the save point the scene is clean again"
    );
    assert!(h.entity_of(alpha).is_some(), "Alpha is back with its GUID");

    for (step, expected) in history.iter().enumerate().skip(1) {
        assert!(h.redo(), "redo step {step} is available");
        assert_eq!(&h.snapshot(), expected, "redo to state {step}");
    }
    assert!(h.is_dirty(), "after redo the scene is dirty again");
    assert!(
        h.entity_of(copy).is_none(),
        "the deleted copy stays deleted after redo"
    );
    assert_eq!(h.parent_of(beta), Some(alpha), "hierarchy is intact");
}

#[test]
fn a_deleted_entity_comes_back_with_its_guid_and_components() {
    let mut h = EditorHarness::new();
    h.new_project("delete_undo", "empty")
        .expect("the project is created");
    h.new_scene("scenes/delete.scene.ron")
        .expect("the scene is created");
    let crate_guid = h.create_entity("Crate").expect("Crate is created");
    h.move_to(crate_guid, [4.0, 5.0, 6.0]).expect("Crate moves");
    let before = h.snapshot();

    h.delete(crate_guid).expect("Crate is deleted");
    assert!(h.entity_of(crate_guid).is_none());

    assert!(h.undo(), "the delete is undoable");
    assert_eq!(
        h.snapshot(),
        before,
        "Crate is back with the same components"
    );
    assert_eq!(h.name_of(crate_guid).as_deref(), Some("Crate"));
}
