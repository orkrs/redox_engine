//! `#[reflect(...)]` metadata written by the derive, and the defaults of fields without it.

use redox_ecs::World;
use redox_reflect::meta::{FieldMeta, TypeMeta, WidgetHint};
use redox_reflect::{
    FieldError, Reflect, ReflectError, ReflectField, ReflectSchema, ReflectValue, TypeRegistry,
};

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(
    category = "Physics",
    tooltip = "Rigid body settings",
    requires("Transform", "Collider"),
    version = 2,
    editor_only
)]
struct Everything {
    #[reflect(
        label = "Mass",
        category = "Body",
        tooltip = "Mass in kilograms",
        range(min = 0.0, max = 1000.0),
        step = 0.5,
        unit = "kg",
        default = 1.0,
        widget = "slider"
    )]
    mass: f32,
    #[reflect(read_only, hidden, advanced, editor_only, widget = "color")]
    tint: f32,
    #[reflect(default = true)]
    enabled: bool,
    #[reflect(default = -2)]
    offset: i32,
    #[reflect(default = "none", widget = "multiline")]
    note: String,
    #[reflect(skip)]
    cache: Vec<u8>,
    plain: u32,
}

#[derive(Reflect, Default)]
struct Plain {
    value: f32,
}

fn field<'a>(
    schema: &'a redox_reflect::ReflectSchema,
    name: &str,
) -> &'a redox_reflect::ReflectField {
    schema
        .fields
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no field {name}"))
}

#[test]
fn attributes_fill_field_metadata() {
    let schema = Everything::reflect_schema();
    assert_eq!(
        field(&schema, "mass").meta,
        FieldMeta {
            label: Some("Mass"),
            category: Some("Body"),
            tooltip: Some("Mass in kilograms"),
            range: Some((0.0, 1000.0)),
            step: Some(0.5),
            unit: Some("kg"),
            default: Some(ReflectValue::F64(1.0)),
            read_only: false,
            hidden: false,
            advanced: false,
            editor_only: false,
            widget: WidgetHint::Slider,
        }
    );
}

#[test]
fn flags_and_widget_are_recorded() {
    let schema = Everything::reflect_schema();
    let tint = field(&schema, "tint").meta.clone();
    assert!(tint.read_only);
    assert!(tint.hidden);
    assert!(tint.advanced);
    assert!(tint.editor_only);
    assert_eq!(tint.widget, WidgetHint::Color);
    assert_eq!(field(&schema, "enabled").meta.default, Some(ReflectValue::Bool(true)));
    assert_eq!(
        field(&schema, "offset").meta.default,
        Some(ReflectValue::I64(-2))
    );
    let note = &field(&schema, "note").meta;
    assert_eq!(note.default, Some(ReflectValue::String("none".into())));
    assert_eq!(note.widget, WidgetHint::Multiline);
}

#[test]
fn attributes_fill_type_metadata() {
    let meta = Everything::reflect_schema().meta;
    assert_eq!(
        meta,
        TypeMeta {
            category: Some("Physics"),
            tooltip: Some("Rigid body settings"),
            requires: &["Transform", "Collider"],
            version: 2,
            editor_only: true,
        }
    );
}

#[test]
fn fields_without_attributes_get_default_metadata() {
    let schema = Everything::reflect_schema();
    let plain = field(&schema, "plain");
    assert_eq!(plain.meta, FieldMeta::default());
    assert_eq!(plain.type_name, "u32");
}

#[test]
fn types_without_attributes_get_version_one() {
    let meta = Plain::reflect_schema().meta;
    assert_eq!(meta, TypeMeta::default());
    assert_eq!(meta.version, 1);
    assert!(meta.requires.is_empty());
}

#[test]
fn skip_still_hides_fields_from_schema() {
    let names: Vec<&str> = Everything::reflect_schema()
        .fields
        .iter()
        .map(|f| f.name)
        .collect();
    assert_eq!(names, ["mass", "tint", "enabled", "offset", "note", "plain"]);
    assert!(Everything::default().to_value().field("cache").is_none());
}

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
struct Tuning {
    #[reflect(range(min = 0.0, max = 1.0))]
    amount: f32,
    #[reflect(step = 0.5)]
    stepped: f32,
    #[reflect(read_only)]
    id: u32,
}

fn tuning_world() -> (TypeRegistry, World, redox_ecs::Entity) {
    let mut registry = TypeRegistry::new();
    registry.register::<Tuning>();
    let mut world = World::new();
    let entity = world.spawn();
    world.add_component(
        entity,
        Tuning {
            amount: 0.5,
            stepped: 0.0,
            id: 7,
        },
    );
    (registry, world, entity)
}

#[test]
fn values_outside_the_range_are_clamped_to_the_boundary() {
    let (registry, mut world, entity) = tuning_world();
    registry
        .set_field(&mut world, entity, "Tuning", "amount", &ReflectValue::F64(5.0))
        .unwrap();
    assert_eq!(world.get_component::<Tuning>(entity).unwrap().amount, 1.0);
    registry
        .set_field(&mut world, entity, "Tuning", "amount", &ReflectValue::F64(-3.0))
        .unwrap();
    assert_eq!(world.get_component::<Tuning>(entity).unwrap().amount, 0.0);
}

#[test]
fn numbers_are_rounded_to_the_step() {
    let (registry, mut world, entity) = tuning_world();
    registry
        .set_field(&mut world, entity, "Tuning", "stepped", &ReflectValue::F64(1.3))
        .unwrap();
    assert_eq!(world.get_component::<Tuning>(entity).unwrap().stepped, 1.5);
}

#[test]
fn read_only_fields_refuse_writes() {
    let (registry, mut world, entity) = tuning_world();
    let result = registry.set_field(&mut world, entity, "Tuning", "id", &ReflectValue::I64(9));
    assert_eq!(result, Err(FieldError::ReadOnly));
    assert_eq!(world.get_component::<Tuning>(entity).unwrap().id, 7);
}

#[test]
fn wrong_shape_is_a_type_mismatch() {
    let (registry, mut world, entity) = tuning_world();
    let result = registry.set_field(
        &mut world,
        entity,
        "Tuning",
        "amount",
        &ReflectValue::Bool(true),
    );
    assert_eq!(
        result,
        Err(FieldError::TypeMismatch {
            expected: "f32".into()
        })
    );
}

#[test]
fn unknown_type_field_and_missing_component_are_errors() {
    let (registry, mut world, entity) = tuning_world();
    let value = ReflectValue::F64(0.1);
    assert_eq!(
        registry.set_field(&mut world, entity, "Nope", "amount", &value),
        Err(FieldError::UnknownType("Nope".into()))
    );
    assert_eq!(
        registry.set_field(&mut world, entity, "Tuning", "missing", &value),
        Err(FieldError::UnknownField("missing".into()))
    );
    let bare = world.spawn();
    assert_eq!(
        registry.set_field(&mut world, bare, "Tuning", "amount", &value),
        Err(FieldError::MissingComponent("Tuning".into()))
    );
}

#[test]
fn field_meta_validate_clamps_floats_and_integers() {
    let float_meta = FieldMeta {
        range: Some((0.0, 1.0)),
        ..Default::default()
    };
    assert_eq!(
        float_meta.validate(&ReflectValue::F64(2.0)),
        Ok(ReflectValue::F64(1.0))
    );
    let int_meta = FieldMeta {
        range: Some((0.0, 10.0)),
        ..Default::default()
    };
    assert_eq!(
        int_meta.validate(&ReflectValue::I64(12)),
        Ok(ReflectValue::I64(10))
    );
}

#[derive(Default, Clone, Debug, PartialEq)]
struct Tuned {
    gain: f32,
}

impl Reflect for Tuned {
    fn type_name() -> &'static str {
        "Tuned"
    }

    fn to_value(&self) -> ReflectValue {
        ReflectValue::Struct {
            type_name: "Tuned".into(),
            fields: vec![("gain".into(), ReflectValue::f32(self.gain))],
        }
    }

    fn apply(&mut self, value: &ReflectValue) -> Result<(), ReflectError> {
        if let Some(gain) = value.field("gain").and_then(ReflectValue::as_f32) {
            self.gain = gain;
        }
        Ok(())
    }

    fn reflect_schema() -> ReflectSchema {
        ReflectSchema {
            type_name: "Tuned",
            fields: vec![ReflectField {
                name: "gain",
                type_name: "f32",
                ..Default::default()
            }],
            meta: TypeMeta {
                version: 2,
                ..Default::default()
            },
        }
    }

    /// Version 1 stored the value as `volume`.
    fn migrate(from: u32, value: &mut ReflectValue) {
        if from == 1
            && let ReflectValue::Struct { fields, .. } = value
        {
            for (name, _) in fields.iter_mut() {
                if name.as_str() == "volume" {
                    *name = "gain".into();
                }
            }
        }
    }
}

#[test]
fn version_one_values_are_migrated_on_load() {
    let mut registry = TypeRegistry::new();
    registry.register::<Tuned>();
    assert_eq!(registry.version("Tuned"), Some(2));

    let mut old = ReflectValue::Struct {
        type_name: "Tuned".into(),
        fields: vec![("volume".into(), ReflectValue::f32(0.5))],
    };
    registry.migrate_value("Tuned", 1, &mut old).unwrap();

    let mut world = World::new();
    let entity = world.spawn();
    registry.insert_component(&mut world, entity, &old).unwrap();
    assert_eq!(
        world.get_component::<Tuned>(entity),
        Some(&Tuned { gain: 0.5 })
    );
}

#[test]
fn current_version_values_are_not_migrated() {
    let mut registry = TypeRegistry::new();
    registry.register::<Tuned>();
    let mut value = Tuned { gain: 0.25 }.to_value();
    let before = value.clone();
    registry.migrate_value("Tuned", 2, &mut value).unwrap();
    assert_eq!(value, before);
    assert!(registry.migrate_value("Nope", 1, &mut value).is_err());
}
