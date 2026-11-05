//! Entity identity: `create_entity` and `duplicate` give every new entity its own `EntityGuid`.

use redox_editor_core::EditorContext;
use redox_math::{Transform, Vec3};
use redox_reflect::TypeRegistry;
use redox_scene::EntityGuid;

fn context() -> EditorContext {
    EditorContext::new(TypeRegistry::new())
}

#[test]
fn create_entity_gives_the_new_entity_a_guid() {
    let ctx = context();

    assert!(ctx.commands.run(&ctx, "create_entity"));

    let entity = ctx.selection.lock().primary.expect("the new entity is selected");
    let world = ctx.world.lock();
    assert!(world.get_component::<EntityGuid>(entity).is_some());
}

#[test]
fn duplicate_gets_a_new_guid_different_from_the_source() {
    let ctx = context();
    let source = {
        let mut world = ctx.world.lock();
        let entity = world.spawn();
        world.add_component(entity, Transform::from_translation(Vec3::ZERO));
        world.add_component(entity, EntityGuid::new_random());
        entity
    };
    ctx.selection.lock().set_primary(Some(source));
    let source_guid = ctx
        .world
        .lock()
        .get_component::<EntityGuid>(source)
        .copied()
        .expect("source guid");

    assert!(ctx.commands.run(&ctx, "duplicate"));

    let copy = ctx.selection.lock().primary.expect("the copy is selected");
    assert_ne!(copy, source);
    let copy_guid = ctx
        .world
        .lock()
        .get_component::<EntityGuid>(copy)
        .copied()
        .expect("copy guid");
    assert_ne!(copy_guid, source_guid);
    assert_eq!(
        ctx.world.lock().get_component::<EntityGuid>(source).copied(),
        Some(source_guid),
        "the source keeps its guid"
    );
}
