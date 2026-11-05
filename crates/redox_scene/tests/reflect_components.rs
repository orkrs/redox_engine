//! Reflected forms of `Name` and `Tag`: value roundtrip, forward compatibility, RON.

use redox_ecs::World;
use redox_reflect::{Reflect, ReflectValue, TypeRegistry};
use redox_scene::{Name, Tag};

#[test]
fn name_value_roundtrips() {
    let original = Name("hero".into());
    let value = original.to_value();

    let mut restored = Name::default();
    restored.apply(&value).expect("apply");

    assert_eq!(restored, original);
    assert_eq!(restored.to_value(), value);
}

#[test]
fn tag_value_roundtrips() {
    let original = Tag("enemy".into());
    let value = original.to_value();

    let mut restored = Tag::default();
    restored.apply(&value).expect("apply");

    assert_eq!(restored, original);
    assert_eq!(restored.to_value(), value);
}

#[test]
fn unknown_fields_are_ignored_by_name_and_tag() {
    let value = ReflectValue::Struct {
        type_name: "Name".into(),
        fields: vec![
            ("name".into(), ReflectValue::String("door".into())),
            ("from_the_future".into(), ReflectValue::I64(7)),
        ],
    };
    let mut name = Name::default();
    name.apply(&value).expect("apply must not fail on extra fields");
    assert_eq!(name, Name("door".into()));

    let tag_value = ReflectValue::Struct {
        type_name: "Tag".into(),
        fields: vec![("unexpected".into(), ReflectValue::Bool(true))],
    };
    let mut tag = Tag("keep".into());
    tag.apply(&tag_value).expect("apply");
    assert_eq!(tag, Tag("keep".into()), "a value without the field leaves it unchanged");
}

#[test]
fn name_value_survives_ron() {
    let value = Name("lamp".into()).to_value();
    let text = ron::ser::to_string(&value).expect("serialize");
    let back: ReflectValue = ron::from_str(&text).expect("parse");
    assert_eq!(back, value);
}

#[test]
fn name_and_tag_are_restored_through_the_registry() {
    let mut registry = TypeRegistry::new();
    registry.register::<Name>();
    registry.register::<Tag>();

    let mut world = World::new();
    let entity = world.spawn();
    world.add_component(entity, Name("crate".into()));
    world.add_component(entity, Tag("prop".into()));

    let text = registry.to_ron(&world).expect("to_ron");
    let mut restored = World::new();
    registry.from_ron(&mut restored, &text).expect("from_ron");

    let found = restored
        .all_entities()
        .next()
        .expect("one restored entity");
    assert_eq!(
        restored.get_component::<Name>(found),
        Some(&Name("crate".into()))
    );
    assert_eq!(
        restored.get_component::<Tag>(found),
        Some(&Tag("prop".into()))
    );
}
