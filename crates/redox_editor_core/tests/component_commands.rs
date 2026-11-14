//! Add, remove and copy component commands with undo transactions.

use redox_ecs::{Entity, World};
use redox_editor_core::component_catalog::DependencyPromptAnswer;
use redox_editor_core::component_commands::{
    bulk_add_component, component_add, component_copy_values, component_remove,
    record_component_transaction, ComponentAddResult,
};
use redox_editor_core::inspector_model::InspectorModel;
use redox_editor_core::{UndoCommand, UndoStack};
use redox_math::{Transform, Vec3};
use redox_reflect::{Reflect, ReflectValue, TypeRegistry, register_engine_defaults};

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(category = "Test")]
struct Marker {
    weight: f32,
}

fn registry() -> TypeRegistry {
    let mut registry = TypeRegistry::new();
    register_engine_defaults(&mut registry);
    registry.register::<Marker>();
    registry
}

fn spawn(world: &mut World) -> Entity {
    let entity = world.spawn();
    world.add_component(entity, redox_scene::EntityGuid::new_random());
    entity
}

fn spawn_transform(world: &mut World) -> Entity {
    let entity = spawn(world);
    world.add_component(entity, Transform::from_translation(Vec3::ZERO));
    entity
}

#[test]
fn add_then_undo_removes_the_component() {
    let registry = registry();
    let mut world = World::new();
    let entity = spawn_transform(&mut world);

    let ComponentAddResult::Ready { commands, .. } = component_add(
        &registry,
        &world,
        &[entity],
        "Marker",
        Some(DependencyPromptAnswer::Continue),
    )
    .expect("add")
    else {
        panic!("expected ready add");
    };
    let mut undo = UndoStack::default();
    record_component_transaction(&mut undo, &registry, &mut world, "Add Marker", commands)
        .expect("record");
    assert!(world.get_component::<Marker>(entity).is_some());

    assert!(undo.undo(&mut world, &registry));
    assert!(world.get_component::<Marker>(entity).is_none());
}

#[test]
fn remove_then_undo_restores_value() {
    let registry = registry();
    let mut world = World::new();
    let entity = spawn(&mut world);
    world.add_component(
        entity,
        Marker {
            weight: 2.5,
        },
    );
    let before = registry
        .read_component(&world, entity, "Marker")
        .expect("marker");

    let outcome = component_remove(&registry, &world, &[entity], "Marker").expect("remove");
    let mut undo = UndoStack::default();
    record_component_transaction(
        &mut undo,
        &registry,
        &mut world,
        "Remove Marker",
        outcome.commands,
    )
    .expect("record");
    assert!(registry.read_component(&world, entity, "Marker").is_none());

    assert!(undo.undo(&mut world, &registry));
    let restored = registry
        .read_component(&world, entity, "Marker")
        .expect("restored");
    assert_eq!(restored, before);
}

#[test]
fn copy_values_to_three_entities_is_one_transaction() {
    let registry = registry();
    let mut world = World::new();
    let source = spawn(&mut world);
    world.add_component(
        source,
        Marker {
            weight: 9.0,
        },
    );
    let mut targets = Vec::new();
    for w in [1.0_f32, 2.0, 3.0] {
        let e = spawn(&mut world);
        world.add_component(e, Marker { weight: w });
        targets.push(e);
    }

    let commands =
        component_copy_values(&registry, &world, source, &targets, "Marker").expect("copy");
    assert_eq!(commands.len(), 3);
    let mut undo = UndoStack::default();
    record_component_transaction(&mut undo, &registry, &mut world, "Copy Marker", commands)
        .expect("record");
    assert_eq!(undo.len(), 1);

    for &entity in &targets {
        let value = registry
            .read_component(&world, entity, "Marker")
            .expect("marker");
        assert_eq!(value.field("weight"), Some(&ReflectValue::F64(9.0)));
    }
}

#[test]
fn bulk_add_skips_entities_that_already_have_the_type() {
    let registry = registry();
    let mut world = World::new();
    let a = spawn_transform(&mut world);
    world.add_component(a, Marker::default());
    let b = spawn_transform(&mut world);
    let c = spawn_transform(&mut world);

    let model = InspectorModel::build(&registry, &world, &[a, b, c], "", false);
    let outcome = bulk_add_component(&model, &registry, &world, "Marker").expect("bulk");
    assert_eq!(outcome.add.commands.len(), 2);

    let mut undo = UndoStack::default();
    record_component_transaction(
        &mut undo,
        &registry,
        &mut world,
        "Add Marker",
        outcome.add.commands,
    )
    .expect("record");
    assert_eq!(undo.len(), 1);
}