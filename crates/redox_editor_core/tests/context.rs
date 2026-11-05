//! Headless tests of `EditorContext`: no window, no GPU, no egui.

use redox_ecs::World;
use redox_editor_core::undo::{UndoCommand, capture_world};
use redox_editor_core::{EditorContext, EditorOnly, editor_only_entities};
use redox_math::{Transform, Vec3};
use redox_reflect::{Reflect, TypeRegistry, register_engine_defaults};
use redox_scene::components::EntityGuid;

fn context() -> EditorContext {
    let mut registry = TypeRegistry::new();
    register_engine_defaults(&mut registry);
    EditorContext::new(registry)
}

#[test]
fn a_context_is_created_without_a_window() {
    let ctx = context();

    assert_eq!(ctx.world.lock().all_entities().count(), 0);
    let ids = ctx.commands.ids();
    assert!(ids.contains(&"create_entity"));
    assert!(ids.contains(&"duplicate"));
    assert!(ids.contains(&"delete"));
}

#[test]
fn create_entity_adds_an_entity_with_a_transform_and_selects_it() {
    let ctx = context();

    assert!(ctx.commands.run(&ctx, "create_entity"));

    let entity = ctx
        .selection
        .lock()
        .primary
        .expect("the new entity is selected");
    {
        let world = ctx.world.lock();
        assert_eq!(world.all_entities().count(), 1);
        assert!(world.get_component::<Transform>(entity).is_some());
    }
    let expected = format!("[INFO] Created entity #{}", entity.id());
    assert!(ctx.log.lines().iter().any(|line| line.as_str() == expected));
}

#[test]
fn duplicate_copies_the_transform_one_unit_along_x() {
    let ctx = context();
    let source = {
        let mut world = ctx.world.lock();
        let entity = world.spawn();
        world.add_component(
            entity,
            Transform::from_translation(Vec3::new(2.0, 3.0, 4.0)),
        );
        entity
    };
    ctx.selection.lock().set_primary(Some(source));

    assert!(ctx.commands.run(&ctx, "duplicate"));

    let copy = ctx.selection.lock().primary.expect("the copy is selected");
    assert!(copy != source);
    let world = ctx.world.lock();
    let transform = world
        .get_component::<Transform>(copy)
        .copied()
        .expect("the copy has a transform");
    assert_eq!(transform.translation, Vec3::new(3.0, 3.0, 4.0));
}

#[test]
fn delete_removes_the_selected_entity_and_clears_the_selection() {
    let ctx = context();
    assert!(ctx.commands.run(&ctx, "create_entity"));
    let entity = ctx
        .selection
        .lock()
        .primary
        .expect("the new entity is selected");

    assert!(ctx.commands.run(&ctx, "delete"));

    assert_eq!(ctx.world.lock().all_entities().count(), 0);
    assert!(
        ctx.world
            .lock()
            .get_component::<Transform>(entity)
            .is_none()
    );
    let selection = ctx.selection.lock();
    assert!(selection.primary.is_none());
    assert!(selection.set.is_empty());
}

#[test]
fn an_unknown_command_is_logged_and_reported() {
    let ctx = context();

    assert!(!ctx.commands.run(&ctx, "no_such_command"));

    assert!(
        ctx.log
            .lines()
            .iter()
            .any(|line| line.as_str() == "[WARN] Unknown command: no_such_command")
    );
}

#[test]
fn undo_and_redo_restore_the_world_through_the_context() {
    let ctx = context();
    let guid = EntityGuid(7);
    let entity = {
        let mut world = ctx.world.lock();
        let entity = world.spawn();
        world.add_component(entity, guid);
        world.add_component(entity, Transform::from_translation(Vec3::ZERO));
        entity
    };
    let before = {
        let world = ctx.world.lock();
        let registry = ctx.registry.lock();
        capture_world(&world, &registry)
    };
    let old_value = Transform::from_translation(Vec3::ZERO).to_value();
    let new_value = Transform::from_translation(Vec3::new(2.0, 0.0, 0.0)).to_value();
    {
        let mut world = ctx.world.lock();
        let registry = ctx.registry.lock();
        registry
            .insert_component(&mut world, entity, &new_value)
            .expect("the edit applies");
        ctx.undo.lock().push(UndoCommand::SetComponent {
            guid,
            type_name: "Transform".into(),
            before: Some(old_value),
            after: Some(new_value.clone()),
        });
    }

    {
        let mut world = ctx.world.lock();
        let registry = ctx.registry.lock();
        assert!(ctx.undo.lock().undo(&mut world, &registry));
    }
    let after_undo = {
        let world = ctx.world.lock();
        let registry = ctx.registry.lock();
        capture_world(&world, &registry)
    };
    assert_eq!(after_undo, before);

    {
        let mut world = ctx.world.lock();
        let registry = ctx.registry.lock();
        assert!(ctx.undo.lock().redo(&mut world, &registry));
    }
    let translation = ctx
        .world
        .lock()
        .get_component::<Transform>(entity)
        .expect("the entity keeps its transform")
        .translation;
    assert_eq!(translation, Vec3::new(2.0, 0.0, 0.0));
}

#[test]
fn editor_only_entities_are_found() {
    let mut world = World::new();
    let plain = world.spawn();
    world.add_component(plain, Transform::default());
    let marked = world.spawn();
    world.add_component(marked, EditorOnly);

    let found = editor_only_entities(&world);

    assert_eq!(found.len(), 1);
    assert!(found[0] == marked);
}
