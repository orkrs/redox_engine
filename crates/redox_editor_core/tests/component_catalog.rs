//! Component catalog search, categories and soft dependency warnings.

use redox_ecs::{Entity, World};
use redox_editor_core::component_catalog::{
    ComponentCatalog, DependencyPromptAnswer, DependencyPromptOutcome, dependency_warning_for_add,
    resolve_dependency_prompt,
};
use redox_editor_core::inspector_model::InspectorModel;
use redox_math::Transform;
use redox_reflect::{Reflect, ReflectValue, TypeRegistry, register_engine_defaults};

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(category = "Physics", tooltip = "Collision shape editor stub")]
struct ColliderShell {
    radius: f32,
}

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(category = "Physics")]
struct RigidBodyShell {
    mass: f32,
}

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(category = "Gameplay", requires("Transform"))]
struct NeedsTransform {
    value: f32,
}

fn registry() -> TypeRegistry {
    let mut registry = TypeRegistry::new();
    register_engine_defaults(&mut registry);
    registry.register::<ColliderShell>();
    registry.register::<RigidBodyShell>();
    registry.register::<NeedsTransform>();
    registry
}

fn spawn(world: &mut World) -> Entity {
    let entity = world.spawn();
    world.add_component(entity, redox_scene::EntityGuid::new_random());
    entity
}

#[test]
fn search_finds_types_by_category() {
    let registry = registry();
    let hits = ComponentCatalog::search(&registry, "physics", false);
    let names: Vec<&str> = hits.iter().map(|e| e.type_name).collect();
    assert!(names.contains(&"ColliderShell"));
    assert!(names.contains(&"RigidBodyShell"));
    assert!(!names.contains(&"Transform"));
}

#[test]
fn requires_metadata_adds_dependency_on_fixture() {
    let registry = registry();
    let mut world = World::new();
    let entity = spawn(&mut world);
    let model = InspectorModel::build(&registry, &world, &[entity], "", false);

    let outcome = model
        .add_component(&registry, &world, "NeedsTransform")
        .expect("add");

    assert_eq!(outcome.commands.len(), 2);
    assert!(outcome.commands.iter().any(|cmd| matches!(
        cmd,
        redox_editor_core::UndoCommand::SetComponent {
            type_name,
            before: None,
            ..
        } if type_name == "Transform"
    )));
}

#[test]
fn soft_prerequisite_warns_when_rigid_body_missing() {
    let registry = registry();
    let mut world = World::new();
    let entity = spawn(&mut world);

    let warning = dependency_warning_for_add(&registry, &world, &[entity], "Collider")
        .expect("collider needs rigid body");
    assert!(warning.missing.iter().any(|m| m == "RigidBody"));
    assert_eq!(
        resolve_dependency_prompt(DependencyPromptAnswer::Cancel),
        DependencyPromptOutcome::Abort
    );
}
