//! `#[reflect(...)]` metadata written by the derive, and the defaults of fields without it.

use redox_reflect::meta::{FieldMeta, TypeMeta, WidgetHint};
use redox_reflect::{Reflect, ReflectValue};

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
