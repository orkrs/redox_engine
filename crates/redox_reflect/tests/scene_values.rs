//! Forward compatibility of reflected values: unknown fields are ignored, and values survive RON.

use redox_reflect::{Reflect, ReflectValue};

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
struct Gauge {
    level: f32,
    count: u32,
}

#[test]
fn unknown_fields_are_ignored_without_panicking() {
    let mut gauge = Gauge { level: 0.5, count: 3 };
    let value = ReflectValue::Struct {
        type_name: "Gauge".into(),
        fields: vec![
            ("level".into(), ReflectValue::F64(0.75)),
            ("color".into(), ReflectValue::String("red".into())),
            ("nested".into(), ReflectValue::List(vec![ReflectValue::Null])),
        ],
    };

    gauge.apply(&value).expect("apply");

    assert_eq!(gauge, Gauge { level: 0.75, count: 3 });
}

#[test]
fn value_with_unexpected_shapes_leaves_fields_unchanged() {
    let mut gauge = Gauge { level: 0.5, count: 3 };
    let value = ReflectValue::Struct {
        type_name: "Gauge".into(),
        fields: vec![
            ("level".into(), ReflectValue::Bool(true)),
            ("count".into(), ReflectValue::String("many".into())),
        ],
    };

    gauge.apply(&value).expect("apply");

    assert_eq!(gauge, Gauge { level: 0.5, count: 3 });
}

#[test]
fn reflected_value_survives_ron_with_unknown_fields_kept() {
    let value = Gauge { level: 0.25, count: 9 }.to_value();
    let text = ron::ser::to_string(&value).expect("serialize");
    let back: ReflectValue = ron::from_str(&text).expect("parse");
    assert_eq!(back, value);

    let mut gauge = Gauge::default();
    gauge.apply(&back).expect("apply");
    assert_eq!(gauge, Gauge { level: 0.25, count: 9 });
}
