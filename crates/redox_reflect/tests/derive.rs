//! `#[derive(Reflect)]` behaviour.

use redox_math::{Quat, Vec3};
use redox_reflect::{Reflect, ReflectValue, TypeRegistry};

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
struct Health {
    current: f32,
    max: f32,
    regen: bool,
    label: String,
    level: u32,
    offset: Vec3,
    facing: Quat,
    #[reflect(skip)]
    cache: Vec<u8>,
}

#[derive(Reflect, Default)]
#[reflect(name = "Renamed")]
struct Original {
    value: i32,
}

#[test]
fn value_round_trips() {
    let a = Health {
        current: 7.5,
        max: 10.0,
        regen: true,
        label: "boss".into(),
        level: 3,
        offset: Vec3::new(1.0, 2.0, 3.0),
        facing: Quat::from_rotation_y(0.5),
        cache: vec![1, 2, 3],
    };
    let mut b = Health::default();
    b.apply(&a.to_value()).unwrap();
    assert_eq!(b.current, 7.5);
    assert_eq!(b.max, 10.0);
    assert!(b.regen);
    assert_eq!(b.label, "boss");
    assert_eq!(b.level, 3);
    assert_eq!(b.offset, a.offset);
    assert!((b.facing.dot(a.facing) - 1.0).abs() < 1e-6);
    assert!(
        b.cache.is_empty(),
        "skipped fields are neither saved nor restored"
    );
}

#[test]
fn schema_lists_fields_in_declaration_order_without_skipped_ones() {
    let schema = Health::reflect_schema();
    assert_eq!(schema.type_name, "Health");
    let names: Vec<_> = schema.fields.iter().map(|f| f.name).collect();
    assert_eq!(
        names,
        [
            "current", "max", "regen", "label", "level", "offset", "facing"
        ]
    );
    assert_eq!(schema.fields[0].type_name, "f32");
    assert_eq!(schema.fields[5].type_name, "Vec3");
}

#[test]
fn type_name_can_be_overridden() {
    assert_eq!(Original::type_name(), "Renamed");
    assert_eq!(
        Original::default().to_value().field("value"),
        Some(&ReflectValue::I64(0))
    );
}

#[test]
fn wrong_shaped_or_out_of_range_values_leave_fields_untouched() {
    let mut h = Health {
        current: 1.0,
        level: 9,
        ..Default::default()
    };
    let bad = ReflectValue::Struct {
        type_name: "Health".into(),
        fields: vec![
            ("current".into(), ReflectValue::String("nope".into())),
            ("level".into(), ReflectValue::I64(-5)),
            ("max".into(), ReflectValue::I64(4)),
        ],
    };
    h.apply(&bad).unwrap();
    assert_eq!(h.current, 1.0, "string is not a number");
    assert_eq!(h.level, 9, "negative value does not fit u32");
    assert_eq!(h.max, 4.0, "integers convert to floats");
}

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
struct Score {
    points: u32,
}

#[test]
fn derived_types_work_with_the_registry() {
    let mut reg = TypeRegistry::new();
    reg.register::<Score>();
    let mut world = redox_ecs::World::new();
    let e = world.spawn();
    let v = Score { points: 42 }.to_value();
    reg.insert_component(&mut world, e, &v).unwrap();
    assert_eq!(world.get_component::<Score>(e), Some(&Score { points: 42 }));
    let ron = reg.to_ron(&world).unwrap();
    assert!(ron.contains("points"));
}
