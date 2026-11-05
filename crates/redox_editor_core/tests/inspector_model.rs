//! Headless tests of the inspector model: rows, mixed values, filters, edits, reset, copy/paste.

use redox_ecs::{Entity, World};
use redox_editor_core::inspector_model::{
    InspectorModel, Row, RowValue, addable_types, copy_component, paste_component_values,
};
use redox_editor_core::{UndoCommand, UndoStack};
use redox_math::{Transform, Vec3};
use redox_reflect::meta::FieldError;
use redox_reflect::{Reflect, ReflectValue, TypeRegistry, register_engine_defaults};

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(category = "Gameplay", requires("Transform"))]
struct Gauge {
    #[reflect(
        label = "Fill level",
        tooltip = "How full the gauge is",
        range(min = 0.0, max = 1.0),
        default = 0.5
    )]
    level: f32,
    #[reflect(advanced)]
    tuning: f32,
    #[reflect(hidden)]
    secret: u32,
    #[reflect(editor_only)]
    gizmo_tint: u32,
    #[reflect(read_only)]
    serial: u32,
}

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
struct Pose {
    translation: Vec3,
    count: u32,
}

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(editor_only)]
struct EditorMarker {
    flag: bool,
}

fn registry() -> TypeRegistry {
    let mut registry = TypeRegistry::new();
    register_engine_defaults(&mut registry);
    registry.register::<Gauge>();
    registry.register::<Pose>();
    registry.register::<EditorMarker>();
    registry
}

fn spawn_transform(world: &mut World, translation: Vec3) -> Entity {
    let entity = world.spawn();
    world.add_component(entity, Transform::from_translation(translation));
    entity
}

fn spawn_gauge(world: &mut World) -> Entity {
    let entity = world.spawn();
    world.add_component(entity, Gauge::default());
    entity
}

fn row<'a>(model: &'a InspectorModel, component: &str, field: &str) -> &'a Row {
    model
        .sections
        .iter()
        .find(|section| section.component == component)
        .and_then(|section| section.rows.iter().find(|row| row.field == field))
        .unwrap_or_else(|| panic!("no row {component}.{field}"))
}

fn field_names(model: &InspectorModel, component: &str) -> Vec<&'static str> {
    model
        .sections
        .iter()
        .filter(|section| section.component == component)
        .flat_map(|section| section.rows.iter().map(|row| row.field))
        .collect()
}

fn apply_all(commands: &[UndoCommand], world: &mut World, registry: &TypeRegistry) {
    for command in commands {
        command.apply(world, registry);
    }
}

fn translation_of(world: &World, entity: Entity) -> Vec3 {
    world
        .get_component::<Transform>(entity)
        .expect("transform")
        .translation
}

fn level_of(world: &World, entity: Entity) -> f32 {
    world.get_component::<Gauge>(entity).expect("gauge").level
}

#[test]
fn a_single_entity_shows_its_components_and_fields() {
    let registry = registry();
    let mut world = World::new();
    let entity = spawn_transform(&mut world, Vec3::new(1.0, 2.0, 3.0));

    let model = InspectorModel::build(&registry, &world, &[entity], "", false);

    let translation = row(&model, "Transform", "translation");
    assert!(!translation.mixed);
    assert_eq!(
        translation.value,
        RowValue::Single(ReflectValue::vec3(1.0, 2.0, 3.0))
    );
    assert_eq!(translation.unit, Some("m"));
    assert!(model.partial_components.is_empty());
}

#[test]
fn two_entities_with_different_values_show_mixed_and_one_undo_restores_both() {
    let registry = registry();
    let mut world = World::new();
    let a = spawn_transform(&mut world, Vec3::new(1.0, 0.0, 0.0));
    let b = spawn_transform(&mut world, Vec3::new(5.0, 0.0, 0.0));
    let model = InspectorModel::build(&registry, &world, &[a, b], "", false);
    let translation = row(&model, "Transform", "translation");
    assert!(translation.mixed);
    assert_eq!(translation.value, RowValue::Mixed);

    let commands = model
        .edit(
            &registry,
            &world,
            "Transform",
            "translation",
            ReflectValue::vec3(2.0, 3.0, 4.0),
        )
        .expect("edit");
    assert_eq!(commands.len(), 2);
    apply_all(&commands, &mut world, &registry);
    let mut undo = UndoStack::default();
    undo.push(UndoCommand::Batch(commands));

    assert_eq!(translation_of(&world, a), Vec3::new(2.0, 3.0, 4.0));
    assert_eq!(translation_of(&world, b), Vec3::new(2.0, 3.0, 4.0));
    assert!(undo.undo(&mut world, &registry));
    assert_eq!(translation_of(&world, a), Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(translation_of(&world, b), Vec3::new(5.0, 0.0, 0.0));
    assert!(!undo.can_undo());
}

#[test]
fn filter_matches_labels_and_tooltips_and_hides_other_rows() {
    let registry = registry();
    let mut world = World::new();
    let entity = spawn_gauge(&mut world);

    let by_tooltip = InspectorModel::build(&registry, &world, &[entity], "HOW FULL", false);
    assert_eq!(field_names(&by_tooltip, "Gauge"), ["level"]);
    assert!(by_tooltip.sections.iter().all(|s| s.component == "Gauge"));

    let by_label = InspectorModel::build(&registry, &world, &[entity], "fill level", false);
    assert_eq!(field_names(&by_label, "Gauge"), ["level"]);

    let nothing = InspectorModel::build(&registry, &world, &[entity], "no such text", false);
    assert!(nothing.sections.is_empty());
}

#[test]
fn hidden_and_editor_fields_never_show_and_advanced_needs_the_toggle() {
    let registry = registry();
    let mut world = World::new();
    let entity = spawn_gauge(&mut world);

    let plain = InspectorModel::build(&registry, &world, &[entity], "", false);
    assert_eq!(field_names(&plain, "Gauge"), ["level", "serial"]);

    let advanced = InspectorModel::build(&registry, &world, &[entity], "", true);
    assert_eq!(
        field_names(&advanced, "Gauge"),
        ["level", "tuning", "serial"]
    );
}

#[test]
fn reset_restores_the_metadata_default_and_the_type_default() {
    let registry = registry();
    let mut world = World::new();
    let gauge = spawn_gauge(&mut world);
    let moved = spawn_transform(&mut world, Vec3::new(1.0, 1.0, 1.0));
    let entities = [gauge, moved];

    let model = InspectorModel::build(&registry, &world, &entities, "", false);
    let level_edit = model
        .edit(&registry, &world, "Gauge", "level", ReflectValue::F64(0.9))
        .expect("edit level");
    apply_all(&level_edit, &mut world, &registry);
    assert_eq!(level_of(&world, gauge), 0.9);
    let reset = model
        .reset_field(&registry, &world, "Gauge", "level")
        .expect("reset level");
    apply_all(&reset, &mut world, &registry);
    assert_eq!(level_of(&world, gauge), 0.5);

    let translation_edit = model
        .edit(
            &registry,
            &world,
            "Transform",
            "translation",
            ReflectValue::vec3(4.0, 4.0, 4.0),
        )
        .expect("edit translation");
    apply_all(&translation_edit, &mut world, &registry);
    let reset = model
        .reset_field(&registry, &world, "Transform", "translation")
        .expect("reset translation");
    apply_all(&reset, &mut world, &registry);
    assert_eq!(translation_of(&world, moved), Transform::default().translation);
}

#[test]
fn values_outside_the_range_are_clamped_by_an_edit() {
    let registry = registry();
    let mut world = World::new();
    let entity = spawn_gauge(&mut world);
    let model = InspectorModel::build(&registry, &world, &[entity], "", false);

    let commands = model
        .edit(&registry, &world, "Gauge", "level", ReflectValue::F64(4.0))
        .expect("edit");
    apply_all(&commands, &mut world, &registry);

    assert_eq!(level_of(&world, entity), 1.0);
}

#[test]
fn read_only_fields_refuse_edits() {
    let registry = registry();
    let mut world = World::new();
    let entity = spawn_gauge(&mut world);
    let model = InspectorModel::build(&registry, &world, &[entity], "", true);

    let result = model.edit(&registry, &world, "Gauge", "serial", ReflectValue::I64(3));
    assert_eq!(result.unwrap_err(), FieldError::ReadOnly);
}

#[test]
fn modified_from_default_is_set_only_when_the_value_differs() {
    let registry = registry();
    let mut world = World::new();
    let gauge = spawn_gauge(&mut world);
    let model = InspectorModel::build(&registry, &world, &[gauge], "", false);
    assert!(!row(&model, "Gauge", "level").modified_from_default);

    let commands = model
        .edit(&registry, &world, "Gauge", "level", ReflectValue::F64(0.9))
        .expect("edit");
    apply_all(&commands, &mut world, &registry);
    let model = InspectorModel::build(&registry, &world, &[gauge], "", false);
    assert!(row(&model, "Gauge", "level").modified_from_default);

    let moved = spawn_transform(&mut world, Vec3::new(1.0, 0.0, 0.0));
    let model = InspectorModel::build(&registry, &world, &[moved], "", false);
    assert!(row(&model, "Transform", "translation").modified_from_default);
}

#[test]
fn paste_copies_only_fields_with_matching_name_and_type() {
    let registry = registry();
    let mut world = World::new();
    let source_entity = spawn_transform(&mut world, Vec3::new(7.0, 8.0, 9.0));
    let target = world.spawn();
    world.add_component(
        target,
        Pose {
            translation: Vec3::ZERO,
            count: 3,
        },
    );
    let source = copy_component(&registry, &world, source_entity, "Transform").expect("copied");

    let commands =
        paste_component_values(&registry, &world, &[target], "Pose", &source).expect("paste");
    assert_eq!(commands.len(), 1);
    apply_all(&commands, &mut world, &registry);

    let pose = world.get_component::<Pose>(target).expect("pose");
    assert_eq!(pose.translation, Vec3::new(7.0, 8.0, 9.0));
    assert_eq!(pose.count, 3);
}

#[test]
fn components_missing_on_some_entities_are_partial_and_not_shown() {
    let registry = registry();
    let mut world = World::new();
    let a = spawn_transform(&mut world, Vec3::ZERO);
    let b = spawn_gauge(&mut world);

    let model = InspectorModel::build(&registry, &world, &[a, b], "", false);

    assert!(model.sections.is_empty());
    assert_eq!(
        model.partial_components,
        vec!["Gauge".to_string(), "Transform".to_string()]
    );
}

#[test]
fn adding_a_component_also_adds_its_requirement_first() {
    let registry = registry();
    let mut world = World::new();
    let entity = world.spawn();
    let model = InspectorModel::build(&registry, &world, &[entity], "", false);

    let outcome = model.add_component(&registry, &world, "Gauge").expect("add");

    assert_eq!(outcome.commands.len(), 2);
    match &outcome.commands[0] {
        UndoCommand::SetComponent {
            type_name,
            before: None,
            after: Some(_),
            ..
        } => assert_eq!(type_name, "Transform"),
        other => panic!("requirement must come first, got {other:?}"),
    }
    assert_eq!(outcome.notes.len(), 1);
    apply_all(&outcome.commands, &mut world, &registry);
    assert!(world.get_component::<Transform>(entity).is_some());
    assert!(world.get_component::<Gauge>(entity).is_some());

    // Undo of an add is the inverse command, which clears the component. Actually removing it
    // needs the RemoveComponent support of EDT-A3.
    for command in outcome.commands.iter().map(UndoCommand::invert) {
        match command {
            UndoCommand::SetComponent { after, .. } => assert!(after.is_none()),
            other => panic!("unexpected inverse {other:?}"),
        }
    }
}

#[test]
fn an_existing_requirement_is_not_added_again() {
    let registry = registry();
    let mut world = World::new();
    let entity = spawn_transform(&mut world, Vec3::ZERO);
    let model = InspectorModel::build(&registry, &world, &[entity], "", false);

    let outcome = model.add_component(&registry, &world, "Gauge").expect("add");

    assert_eq!(outcome.commands.len(), 1);
    assert!(outcome.notes.is_empty());
}

#[test]
fn removing_a_required_component_warns_but_is_allowed() {
    let registry = registry();
    let mut world = World::new();
    let entity = spawn_transform(&mut world, Vec3::ZERO);
    world.add_component(entity, Gauge::default());
    let model = InspectorModel::build(&registry, &world, &[entity], "", false);

    let outcome = model
        .remove_component(&registry, &world, "Transform")
        .expect("remove");

    assert_eq!(outcome.commands.len(), 1);
    assert_eq!(outcome.warnings.len(), 1);
    assert!(outcome.warnings[0].contains("Gauge"));
}

#[test]
fn add_menu_skips_present_types_and_editor_only_unless_asked() {
    let registry = registry();
    let mut world = World::new();
    let entity = spawn_transform(&mut world, Vec3::ZERO);

    let plain = addable_types(&registry, &world, &[entity], "", false);
    let names: Vec<&str> = plain.iter().map(|t| t.type_name).collect();
    assert!(!names.contains(&"Transform"));
    assert!(names.contains(&"Gauge"));
    assert!(!names.contains(&"EditorMarker"));

    let with_editor = addable_types(&registry, &world, &[entity], "", true);
    assert!(with_editor.iter().any(|t| t.type_name == "EditorMarker"));

    let by_category = addable_types(&registry, &world, &[entity], "gameplay", false);
    let names: Vec<&str> = by_category.iter().map(|t| t.type_name).collect();
    assert_eq!(names, ["Gauge"]);
}
