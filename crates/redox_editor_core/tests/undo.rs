//! Headless tests of the undo stack: identity by GUID, component add and remove, errors,
//! transactions and merging. No window and no GPU.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use redox_ecs::World;
use redox_ecs::hierarchy::{Parent, set_parent};
use redox_editor_core::builtin_commands::record_spawn;
use redox_editor_core::{EditorContext, GuidResolver};
use redox_editor_core::undo::{Clock, UndoCommand, UndoError, UndoStack, capture_entity};
use redox_math::{Transform, Vec3};
use redox_reflect::{Reflect, TypeRegistry};
use redox_scene::components::{EntityGuid, Name};

/// Clock whose time moves only when a test advances it.
struct ManualClock {
    now: Mutex<Instant>,
}

impl ManualClock {
    fn new() -> Self {
        Self {
            now: Mutex::new(Instant::now()),
        }
    }

    fn advance(&self, by: Duration) {
        *self.now.lock().expect("clock lock") += by;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        *self.now.lock().expect("clock lock")
    }
}

fn registry() -> TypeRegistry {
    let mut registry = TypeRegistry::new();
    registry.register::<Transform>();
    registry
}

fn transform_x(value: f32) -> redox_reflect::ReflectValue {
    Transform::from_translation(Vec3::new(value, 0.0, 0.0)).to_value()
}

fn translation_x(world: &World, guid: EntityGuid) -> Option<f32> {
    let entity = GuidResolver::resolve(world, guid)?;
    world
        .get_component::<Transform>(entity)
        .map(|t| t.translation.x)
}

/// Spawns an entity with a GUID and a transform at x = `x`.
fn spawn_with_transform(world: &mut World, guid: EntityGuid, x: f32) {
    let entity = world.spawn();
    world.add_component(entity, guid);
    world.add_component(entity, Transform::from_translation(Vec3::new(x, 0.0, 0.0)));
}

/// Sets the entity's transform x to `to`, applying the change and recording it on `undo`.
fn edit_x(
    registry: &TypeRegistry,
    world: &mut World,
    undo: &mut UndoStack,
    guid: EntityGuid,
    to: f32,
) {
    let entity = GuidResolver::resolve(world, guid).expect("the entity exists");
    let before = registry.read_component(world, entity, "Transform");
    let after = transform_x(to);
    registry
        .insert_component(world, entity, &after)
        .expect("the edit applies");
    undo.push(UndoCommand::SetComponent {
        guid,
        type_name: "Transform".into(),
        before,
        after: Some(after),
    });
}

#[test]
fn deleted_entity_comes_back_with_its_guid_and_later_commands_still_apply() {
    let registry = registry();
    let mut world = World::new();
    let mut undo = UndoStack::default();
    let guid = EntityGuid(1);

    // Create A at x = 0.
    let a = world.spawn();
    world.add_component(a, guid);
    world.add_component(a, Transform::from_translation(Vec3::ZERO));
    undo.push(UndoCommand::Spawn {
        guid,
        snapshot: capture_entity(&world, a, &registry),
    });

    // Edit A.Transform to x = 2.
    registry
        .insert_component(&mut world, a, &transform_x(2.0))
        .expect("the edit applies");
    undo.push(UndoCommand::SetComponent {
        guid,
        type_name: "Transform".into(),
        before: Some(transform_x(0.0)),
        after: Some(transform_x(2.0)),
    });

    // Delete A.
    let snapshot = capture_entity(&world, a, &registry);
    undo.push(UndoCommand::Despawn { guid, snapshot });
    world.despawn(a);
    assert_eq!(translation_x(&world, guid), None);

    // Undo the delete: A is back with the same GUID and its edited transform.
    assert!(undo.undo(&mut world, &registry));
    assert_eq!(translation_x(&world, guid), Some(2.0));

    // Undo the edit: the old transform is back, and the command resolved A by its GUID.
    assert!(undo.undo(&mut world, &registry));
    assert_eq!(translation_x(&world, guid), Some(0.0));

    // Redo the edit, then the delete.
    assert!(undo.redo(&mut world, &registry));
    assert_eq!(translation_x(&world, guid), Some(2.0));
    assert!(undo.redo(&mut world, &registry));
    assert_eq!(translation_x(&world, guid), None);

    // Undo the delete once more.
    assert!(undo.undo(&mut world, &registry));
    assert_eq!(translation_x(&world, guid), Some(2.0));
    assert_eq!(undo.last_error, None);
}

#[test]
fn added_component_is_removed_by_undo_and_restored_by_redo() {
    let registry = registry();
    let mut world = World::new();
    let mut undo = UndoStack::default();
    let guid = EntityGuid(2);
    let entity = world.spawn();
    world.add_component(entity, guid);

    let value = transform_x(5.0);
    registry
        .insert_component(&mut world, entity, &value)
        .expect("the component is added");
    undo.push(UndoCommand::SetComponent {
        guid,
        type_name: "Transform".into(),
        before: None,
        after: Some(value),
    });

    assert!(undo.undo(&mut world, &registry));
    assert!(world.get_component::<Transform>(entity).is_none());

    assert!(undo.redo(&mut world, &registry));
    assert_eq!(translation_x(&world, guid), Some(5.0));
}

#[test]
fn removed_component_is_restored_by_undo_with_its_value() {
    let registry = registry();
    let mut world = World::new();
    let mut undo = UndoStack::default();
    let guid = EntityGuid(3);
    spawn_with_transform(&mut world, guid, 3.0);
    let entity = GuidResolver::resolve(&world, guid).expect("the entity exists");

    let before = registry.read_component(&world, entity, "Transform");
    assert!(registry.remove_component(&mut world, entity, "Transform"));
    undo.push(UndoCommand::SetComponent {
        guid,
        type_name: "Transform".into(),
        before,
        after: None,
    });
    assert!(world.get_component::<Transform>(entity).is_none());

    assert!(undo.undo(&mut world, &registry));
    assert_eq!(translation_x(&world, guid), Some(3.0));
}

#[test]
fn command_on_a_missing_entity_returns_an_error_and_stays_in_history() {
    let registry = registry();
    let mut world = World::new();
    let mut undo = UndoStack::default();
    let missing = EntityGuid(99);
    let cmd = UndoCommand::SetComponent {
        guid: missing,
        type_name: "Transform".into(),
        before: None,
        after: Some(transform_x(1.0)),
    };

    assert_eq!(
        cmd.apply(&mut world, &registry),
        Err(UndoError::EntityNotFound(missing))
    );

    undo.push(cmd);
    assert!(!undo.undo(&mut world, &registry));
    assert_eq!(undo.len(), 1);
    assert_eq!(undo.last_error, Some(UndoError::EntityNotFound(missing)));
}

#[test]
fn command_with_an_unregistered_type_returns_an_error() {
    let registry = registry();
    let mut world = World::new();
    let guid = EntityGuid(5);
    spawn_with_transform(&mut world, guid, 0.0);

    let cmd = UndoCommand::SetComponent {
        guid,
        type_name: "NoSuchType".into(),
        before: None,
        after: Some(transform_x(1.0)),
    };

    assert_eq!(
        cmd.apply(&mut world, &registry),
        Err(UndoError::UnknownType("NoSuchType".into()))
    );
}

#[test]
fn nested_transactions_record_one_named_step() {
    let registry = registry();
    let mut world = World::new();
    let mut undo = UndoStack::default();
    let guid = EntityGuid(10);
    spawn_with_transform(&mut world, guid, 0.0);

    undo.begin("Outer");
    undo.begin("Inner");
    edit_x(&registry, &mut world, &mut undo, guid, 1.0);
    assert!(undo.commit());
    assert!(undo.in_transaction());
    assert_eq!(undo.len(), 0);
    edit_x(&registry, &mut world, &mut undo, guid, 2.0);
    assert!(undo.commit());

    assert!(!undo.in_transaction());
    assert_eq!(undo.len(), 1);
    assert_eq!(undo.entries()[0].label, "Outer");
    assert!(undo.undo(&mut world, &registry));
    assert_eq!(translation_x(&world, guid), Some(0.0));
    assert!(!undo.commit());
}

#[test]
fn cancel_reverts_the_world_and_leaves_no_step() {
    let registry = registry();
    let mut world = World::new();
    let mut undo = UndoStack::default();
    let guid = EntityGuid(11);
    spawn_with_transform(&mut world, guid, 0.0);

    undo.begin("Drag");
    edit_x(&registry, &mut world, &mut undo, guid, 5.0);
    edit_x(&registry, &mut world, &mut undo, guid, 7.0);
    assert_eq!(translation_x(&world, guid), Some(7.0));

    assert!(undo.cancel(&mut world, &registry));
    assert_eq!(translation_x(&world, guid), Some(0.0));
    assert_eq!(undo.len(), 0);
    assert!(!undo.in_transaction());
    assert!(!undo.cancel(&mut world, &registry));
}

#[test]
fn rapid_edits_of_one_value_merge_and_later_edits_do_not() {
    let registry = registry();
    let mut world = World::new();
    let clock = Arc::new(ManualClock::new());
    let mut undo = UndoStack::with_clock(100, clock.clone());
    let guid = EntityGuid(12);
    spawn_with_transform(&mut world, guid, 0.0);

    edit_x(&registry, &mut world, &mut undo, guid, 1.0);
    clock.advance(Duration::from_millis(100));
    edit_x(&registry, &mut world, &mut undo, guid, 2.0);
    assert_eq!(undo.len(), 1);

    clock.advance(Duration::from_secs(2));
    edit_x(&registry, &mut world, &mut undo, guid, 3.0);
    assert_eq!(undo.len(), 2);

    assert!(undo.undo(&mut world, &registry));
    assert_eq!(translation_x(&world, guid), Some(2.0));
    assert!(undo.undo(&mut world, &registry));
    assert_eq!(translation_x(&world, guid), Some(0.0));
}

#[test]
fn a_plain_edit_does_not_merge_into_a_transaction() {
    let registry = registry();
    let mut world = World::new();
    let clock = Arc::new(ManualClock::new());
    let mut undo = UndoStack::with_clock(100, clock);
    let guid = EntityGuid(13);
    spawn_with_transform(&mut world, guid, 0.0);

    undo.begin("Transaction");
    edit_x(&registry, &mut world, &mut undo, guid, 1.0);
    assert!(undo.commit());
    edit_x(&registry, &mut world, &mut undo, guid, 2.0);

    assert_eq!(undo.len(), 2);
    assert!(undo.undo(&mut world, &registry));
    assert_eq!(translation_x(&world, guid), Some(1.0));
    assert!(undo.undo(&mut world, &registry));
    assert_eq!(translation_x(&world, guid), Some(0.0));
}

fn parent_of(world: &World, guid: EntityGuid) -> Option<EntityGuid> {
    let entity = GuidResolver::resolve(world, guid)?;
    let parent = world.get_component::<Parent>(entity)?;
    GuidResolver::guid_of(world, parent.0)
}

/// X of the entity in world space; the tests use parents without a parent of their own.
fn world_x(world: &World, guid: EntityGuid) -> Option<f32> {
    let local = translation_x(world, guid)?;
    match parent_of(world, guid) {
        Some(parent) => Some(local + translation_x(world, parent)?),
        None => Some(local),
    }
}

fn approx(actual: Option<f32>, expected: f32) -> bool {
    actual.is_some_and(|value| (value - expected).abs() < 1e-4)
}

fn name_of(world: &World, guid: EntityGuid) -> Option<String> {
    let entity = GuidResolver::resolve(world, guid)?;
    world.get_component::<Name>(entity).map(|name| name.0.clone())
}

#[test]
fn reparent_keeping_the_world_position_is_undone_and_redone() {
    let registry = registry();
    let mut world = World::new();
    let mut undo = UndoStack::default();
    let parent = EntityGuid(20);
    let child = EntityGuid(21);
    spawn_with_transform(&mut world, parent, 10.0);
    spawn_with_transform(&mut world, child, 1.0);

    let cmd = UndoCommand::reparent(&world, child, Some(parent), true).expect("the move is valid");
    cmd.apply(&mut world, &registry).expect("the move applies");
    undo.push(cmd);

    assert!(approx(translation_x(&world, child), -9.0));
    assert!(approx(world_x(&world, child), 1.0));
    assert_eq!(parent_of(&world, child), Some(parent));

    assert!(undo.undo(&mut world, &registry));
    assert!(approx(translation_x(&world, child), 1.0));
    assert_eq!(parent_of(&world, child), None);

    assert!(undo.redo(&mut world, &registry));
    assert!(approx(world_x(&world, child), 1.0));
    assert_eq!(parent_of(&world, child), Some(parent));
}

#[test]
fn reparenting_under_a_descendant_is_rejected_and_changes_nothing() {
    let mut world = World::new();
    let a = EntityGuid(30);
    let b = EntityGuid(31);
    spawn_with_transform(&mut world, a, 0.0);
    spawn_with_transform(&mut world, b, 0.0);
    let a_entity = GuidResolver::resolve(&world, a).expect("a exists");
    let b_entity = GuidResolver::resolve(&world, b).expect("b exists");
    set_parent(&mut world, b_entity, a_entity);

    assert_eq!(
        UndoCommand::reparent(&world, a, Some(b), false).err(),
        Some(UndoError::Cycle)
    );
    assert_eq!(
        UndoCommand::reparent(&world, a, Some(a), false).err(),
        Some(UndoError::Cycle)
    );
    assert_eq!(parent_of(&world, b), Some(a));
    assert_eq!(parent_of(&world, a), None);
}

#[test]
fn rename_is_undone_and_redone() {
    let registry = registry();
    let mut world = World::new();
    let mut undo = UndoStack::default();
    let guid = EntityGuid(40);
    spawn_with_transform(&mut world, guid, 0.0);
    let entity = GuidResolver::resolve(&world, guid).expect("the entity exists");
    world.add_component(entity, Name("Old".into()));

    let cmd = UndoCommand::Rename {
        guid,
        before: Some("Old".into()),
        after: Some("New".into()),
    };
    cmd.apply(&mut world, &registry).expect("the rename applies");
    undo.push(cmd);
    assert_eq!(name_of(&world, guid).as_deref(), Some("New"));

    assert!(undo.undo(&mut world, &registry));
    assert_eq!(name_of(&world, guid).as_deref(), Some("Old"));

    assert!(undo.redo(&mut world, &registry));
    assert_eq!(name_of(&world, guid).as_deref(), Some("New"));
}

fn undo_in(ctx: &EditorContext) -> bool {
    let mut world = ctx.world.lock();
    let registry = ctx.registry.lock();
    ctx.undo.lock().undo(&mut world, &registry)
}

fn redo_in(ctx: &EditorContext) -> bool {
    let mut world = ctx.world.lock();
    let registry = ctx.registry.lock();
    ctx.undo.lock().redo(&mut world, &registry)
}

fn entity_count(ctx: &EditorContext) -> usize {
    ctx.world.lock().all_entities().count()
}

fn primary_guid(ctx: &EditorContext) -> EntityGuid {
    let entity = ctx.selection.lock().primary.expect("an entity is selected");
    GuidResolver::guid_of(&ctx.world.lock(), entity).expect("the entity has a guid")
}

#[test]
fn create_entity_is_undone_and_redone_with_the_same_guid() {
    let ctx = EditorContext::new(registry());
    assert!(ctx.commands.run(&ctx, "create_entity"));
    let guid = primary_guid(&ctx);
    assert_eq!(ctx.undo.lock().entries()[0].label, "Create Entity");

    assert!(undo_in(&ctx));
    assert_eq!(entity_count(&ctx), 0);

    assert!(redo_in(&ctx));
    assert!(GuidResolver::resolve(&ctx.world.lock(), guid).is_some());
}

#[test]
fn duplicate_is_one_step_and_undo_removes_the_copy() {
    let ctx = EditorContext::new(registry());
    assert!(ctx.commands.run(&ctx, "create_entity"));
    assert!(ctx.commands.run(&ctx, "duplicate"));
    assert_eq!(entity_count(&ctx), 2);
    assert_eq!(ctx.undo.lock().len(), 2);
    assert_eq!(ctx.undo.lock().entries()[1].label, "Duplicate Entity");

    assert!(undo_in(&ctx));
    assert_eq!(entity_count(&ctx), 1);
}

#[test]
fn delete_is_undone_with_its_components_and_guid() {
    let ctx = EditorContext::new(registry());
    assert!(ctx.commands.run(&ctx, "create_entity"));
    let guid = primary_guid(&ctx);
    assert!(ctx.commands.run(&ctx, "delete"));
    assert!(GuidResolver::resolve(&ctx.world.lock(), guid).is_none());

    assert!(undo_in(&ctx));
    let restored = GuidResolver::resolve(&ctx.world.lock(), guid).expect("the entity is back");
    assert!(ctx.world.lock().get_component::<Transform>(restored).is_some());
    assert_eq!(ctx.undo.lock().redo_entries()[0].label, "Delete Entity");
}

#[test]
fn brush_stroke_of_several_objects_is_undone_by_one_step() {
    let ctx = EditorContext::new(registry());
    ctx.undo.lock().begin("Brush Stroke");
    for offset in 0..5 {
        let entity = {
            let mut world = ctx.world.lock();
            let entity = world.spawn();
            world.add_component(
                entity,
                Transform::from_translation(Vec3::new(offset as f32, 0.0, 0.0)),
            );
            entity
        };
        record_spawn(&ctx, entity);
    }
    assert!(ctx.undo.lock().commit());
    assert_eq!(ctx.undo.lock().len(), 1);
    assert_eq!(ctx.undo.lock().entries()[0].label, "Brush Stroke");
    assert_eq!(entity_count(&ctx), 5);

    assert!(undo_in(&ctx));
    assert_eq!(entity_count(&ctx), 0);
    assert!(redo_in(&ctx));
    assert_eq!(entity_count(&ctx), 5);
}

/// Records a step that changes nothing; used where only the history shape matters.
fn push_step(undo: &mut UndoStack) {
    undo.push(UndoCommand::Batch(Vec::new()));
}

#[test]
fn a_new_history_is_clean_and_an_edit_makes_it_dirty() {
    let mut undo = UndoStack::default();
    assert!(!undo.is_dirty());

    push_step(&mut undo);
    assert!(undo.is_dirty());
}

#[test]
fn undo_back_to_the_saved_step_is_clean_and_redo_is_dirty() {
    let registry = registry();
    let mut world = World::new();
    let mut undo = UndoStack::default();
    push_step(&mut undo);
    undo.mark_saved();
    push_step(&mut undo);
    assert!(undo.is_dirty());

    assert!(undo.undo(&mut world, &registry));
    assert!(!undo.is_dirty());
    assert!(undo.redo(&mut world, &registry));
    assert!(undo.is_dirty());
}

#[test]
fn history_cut_off_past_the_saved_state_stays_dirty_until_the_next_save() {
    let registry = registry();
    let mut world = World::new();
    let mut undo = UndoStack::new(2);
    undo.mark_saved();
    push_step(&mut undo);
    push_step(&mut undo);
    assert!(undo.is_dirty());

    // The first step is cut off, so the saved state (the start) is no longer reachable.
    push_step(&mut undo);
    assert_eq!(undo.len(), 2);
    assert!(undo.undo(&mut world, &registry));
    assert!(undo.undo(&mut world, &registry));
    assert!(undo.is_dirty());

    undo.mark_saved();
    assert!(!undo.is_dirty());
}

#[test]
fn a_new_edit_after_undoing_past_the_saved_state_keeps_the_document_dirty() {
    let registry = registry();
    let mut world = World::new();
    let mut undo = UndoStack::default();
    push_step(&mut undo);
    push_step(&mut undo);
    assert!(undo.undo(&mut world, &registry));
    undo.mark_saved();
    assert!(undo.undo(&mut world, &registry));
    assert!(undo.is_dirty());

    // The new edit discards the redo branch that held the saved state.
    push_step(&mut undo);
    assert_eq!(undo.len(), 1);
    assert!(undo.is_dirty());
    assert!(undo.undo(&mut world, &registry));
    assert!(undo.is_dirty());

    undo.mark_saved();
    assert!(!undo.is_dirty());
}

#[test]
fn limit_evicts_the_oldest_steps_and_keeps_the_order() {
    let registry = registry();
    let mut world = World::new();
    let mut undo = UndoStack::new(3);

    for index in 1..=5 {
        undo.begin(format!("Step {index}"));
        undo.push(UndoCommand::Batch(Vec::new()));
        assert!(undo.commit());
    }

    let labels: Vec<&str> = undo.entries().iter().map(|e| e.label.as_str()).collect();
    assert_eq!(labels, ["Step 3", "Step 4", "Step 5"]);
    assert_eq!(undo.limit(), 3);
    assert!(undo.undo(&mut world, &registry));
    assert!(undo.undo(&mut world, &registry));
    assert!(undo.undo(&mut world, &registry));
    assert!(!undo.undo(&mut world, &registry));
}
