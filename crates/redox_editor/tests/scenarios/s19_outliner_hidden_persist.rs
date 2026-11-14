//! EDT-B1 step 5: hiding an entity saves into the scene file and restores after reload.

use redox_reflect::ReflectValue;

use crate::common::EditorHarness;

const SCENE: &str = "scenes/hidden.scene.ron";

#[test]
fn hidden_visibility_survives_save_and_reload() {
    let mut h = EditorHarness::new();
    h.new_project("hidden_outliner", "empty")
        .expect("the project is created");
    h.new_scene(SCENE).expect("the scene is created");
    let entity = h.create_entity("HiddenProp").expect("entity is created");
    h.add_component(
        entity,
        "Visibility",
        vec![("visible", ReflectValue::Bool(false))],
    )
    .expect("entity is hidden");
    h.save().expect("the scene is saved");
    let dir = h.project_dir();

    let mut h = h.restart();
    h.open_project(&dir).expect("project opens");
    h.open_scene(SCENE).expect("scene opens");

    let guid = h.find_by_name("HiddenProp").expect("entity is found");
    let world = h.ctx.world.lock();
    let resolved = redox_editor_core::guid::GuidResolver::resolve(&world, guid).expect("entity");
    let visible = world
        .get_component::<redox_editor_core::Visibility>(resolved)
        .map(|v| v.0)
        .unwrap_or(true);
    assert!(!visible, "hidden flag is restored from the scene file");
}