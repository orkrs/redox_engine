//! TZ §23.2: a scene saved and loaded again gives the same world, GUIDs and hierarchy, and a
//! second save writes the same bytes.

use redox_reflect::ReflectValue;

use crate::common::EditorHarness;

const SCENE: &str = "scenes/roundtrip.scene.ron";

fn f(value: f64) -> ReflectValue {
    ReflectValue::F64(value)
}

#[test]
fn a_saved_scene_loads_back_with_the_same_world_guids_and_hierarchy() {
    let mut h = EditorHarness::new();
    h.new_project("scene_roundtrip", "empty")
        .expect("the project is created");
    h.new_scene(SCENE).expect("the scene is created");

    let ground = h.create_entity("Ground").expect("Ground is created");
    let crate_guid = h.create_entity("Crate").expect("Crate is created");
    let top = h.create_entity("Crate Top").expect("Crate Top is created");
    h.move_to(crate_guid, [1.0, 2.0, 3.0]).expect("Crate moves");
    h.reparent(crate_guid, Some(ground)).expect("Crate gets a parent");
    h.reparent(top, Some(crate_guid)).expect("Crate Top gets a parent");
    h.add_component(
        ground,
        "Camera",
        vec![("fov_y_degrees", f(60.0)), ("near", f(0.1)), ("far", f(500.0))],
    )
    .expect("a camera is added");
    h.add_component(
        crate_guid,
        "DirectionalLight",
        vec![("intensity", f(3.0))],
    )
    .expect("a light is added");
    let before = h.snapshot();
    let mut guids_before = h.all_guids();
    guids_before.sort_by_key(|g| g.0);

    h.save().expect("the scene is saved");
    let first = h.read_file(SCENE);
    h.save().expect("the scene is saved again");
    assert_eq!(h.read_file(SCENE), first, "a second save writes the same bytes");

    let mut h = h.restart();
    let dir = h.root().join("projects/scene_roundtrip");
    h.open_project(&dir).expect("the project opens again");
    h.open_scene(SCENE).expect("the scene opens again");

    assert_eq!(h.snapshot(), before, "the world is the same after reload");
    let mut guids_after = h.all_guids();
    guids_after.sort_by_key(|g| g.0);
    assert_eq!(guids_after, guids_before, "GUIDs are the same after reload");
    assert_eq!(h.parent_of(top), Some(crate_guid), "hierarchy is restored");
    assert_eq!(h.parent_of(crate_guid), Some(ground));

    h.save().expect("the reloaded scene is saved");
    assert_eq!(h.read_file(SCENE), first, "saving the reloaded scene keeps the bytes");
    assert!(h.load_warning_tags().is_empty(), "no warnings on reload");
}
