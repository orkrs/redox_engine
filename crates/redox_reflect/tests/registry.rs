//! Reflection registry: component round trips and RON snapshots.

use redox_ecs::World;
use redox_math::{Quat, Transform, Vec3};
use redox_reflect::{Reflect, ReflectValue, TypeRegistry, register_engine_defaults};

fn registry() -> TypeRegistry {
    let mut r = TypeRegistry::new();
    register_engine_defaults(&mut r);
    r
}

#[test]
fn transform_round_trips_through_reflect_values() {
    let original = Transform {
        translation: Vec3::new(1.0, 2.0, 3.0),
        rotation: Quat::from_rotation_y(0.5),
        scale: Vec3::new(2.0, 2.0, 2.0),
    };
    let value = original.to_value();
    let mut restored = Transform::default();
    restored.apply(&value).unwrap();
    assert_eq!(restored.translation, original.translation);
    assert!((restored.rotation.dot(original.rotation) - 1.0).abs() < 1e-6);
    assert_eq!(restored.scale, original.scale);
}

#[test]
fn registry_reads_and_inserts_components() {
    let reg = registry();
    assert!(reg.type_names().any(|n| n == "Transform"));
    assert!(reg.schema("Transform").is_some());

    let mut world = World::new();
    let e = world.spawn();
    let value = Transform::from_translation(Vec3::new(4.0, 5.0, 6.0)).to_value();
    reg.insert_component(&mut world, e, &value).unwrap();

    let read = reg.read_component(&world, e, "Transform").unwrap();
    assert_eq!(
        read.field("translation").unwrap().as_vec3(),
        Some([4.0, 5.0, 6.0])
    );
    assert!(reg.read_component(&world, e, "Nope").is_none());
}

#[test]
fn unregistered_or_malformed_values_are_errors() {
    let reg = registry();
    let mut world = World::new();
    let e = world.spawn();
    assert!(
        reg.insert_component(&mut world, e, &ReflectValue::Bool(true))
            .is_err()
    );
    let unknown = ReflectValue::Struct {
        type_name: "Mystery".into(),
        fields: vec![],
    };
    assert!(reg.insert_component(&mut world, e, &unknown).is_err());
}

#[test]
fn world_survives_a_ron_round_trip() {
    let reg = registry();
    let mut world = World::new();
    for i in 0..3 {
        let e = world.spawn();
        world.add_component(
            e,
            Transform::from_translation(Vec3::new(i as f32, 0.0, 0.0)),
        );
    }
    let ron = reg.to_ron(&world).unwrap();

    let mut restored = World::new();
    reg.from_ron(&mut restored, &ron).unwrap();
    let mut xs: Vec<f32> = restored
        .query::<&Transform>()
        .iter(&restored)
        .map(|t| t.translation.x)
        .collect();
    xs.sort_by(f32::total_cmp);
    assert_eq!(xs, vec![0.0, 1.0, 2.0]);
    assert!(reg.from_ron(&mut restored, "not ron at all").is_err());
}

#[test]
fn asset_handle_values_carry_an_optional_path() {
    let set = ReflectValue::handle("Mesh", Some("a/b.glb".into()));
    assert_eq!(set.as_handle_path(), Some(Some("a/b.glb".to_string())));
    let unset = ReflectValue::handle("Mesh", None);
    assert_eq!(unset.as_handle_path(), Some(None));
    assert_eq!(ReflectValue::Bool(true).as_handle_path(), None);
}
