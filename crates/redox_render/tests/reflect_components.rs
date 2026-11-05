//! Reflected forms of the render scene components: value roundtrip, forward compatibility, RON.

use redox_math::Vec3;
use redox_reflect::{Reflect, ReflectValue};
use redox_render::{Camera, DirectionalLight, MaterialHandle, MeshHandle, PointLight, SpotLight};

/// Applies `original`'s value to a default instance and checks that the value is unchanged.
fn assert_value_roundtrip<T: Reflect + Default>(original: &T) -> ReflectValue {
    let value = original.to_value();
    let mut restored = T::default();
    restored.apply(&value).expect("apply");
    assert_eq!(restored.to_value(), value);
    value
}

/// Adds an extra field that this build does not know about.
fn with_extra_field(mut value: ReflectValue) -> ReflectValue {
    if let ReflectValue::Struct { fields, .. } = &mut value {
        fields.push(("from_the_future".into(), ReflectValue::I64(42)));
    }
    value
}

#[test]
fn camera_value_roundtrips() {
    let camera = Camera::new(1.0, 1.5, 0.2, 500.0);
    assert_value_roundtrip(&camera);
}

#[test]
fn spot_light_value_roundtrips() {
    let light = SpotLight::new(
        Vec3::new(1.0, 2.0, 3.0),
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(1.0, 0.5, 0.25),
        2.0,
        12.0,
        0.5,
    );
    assert_value_roundtrip(&light);
}

#[test]
fn directional_light_value_roundtrips() {
    // Axis-aligned direction: `apply` normalizes again, and this input is exact under that.
    let light = DirectionalLight::new(Vec3::new(0.0, 1.0, 0.0), Vec3::ONE, 1.5);
    assert_value_roundtrip(&light);
}

#[test]
fn point_light_value_roundtrips() {
    let light = PointLight::new(Vec3::new(1.0, 2.0, 3.0), Vec3::ONE, 5.0, 10.0);
    assert_value_roundtrip(&light);
}

#[test]
fn unset_handles_are_written_without_a_path() {
    let mesh = MeshHandle::default();
    assert_eq!(mesh.to_value().as_handle_path(), Some(None));
    assert_value_roundtrip(&mesh);

    let material = MaterialHandle::default();
    assert_eq!(material.to_value().as_handle_path(), Some(None));
    assert_value_roundtrip(&material);
}

#[test]
fn unknown_fields_are_ignored_by_render_components() {
    let camera = Camera::new(1.0, 1.5, 0.2, 500.0);
    let mut restored = Camera::default();
    restored
        .apply(&with_extra_field(camera.to_value()))
        .expect("apply must not fail on extra fields");
    assert_eq!(restored.to_value(), camera.to_value());

    let light = PointLight::new(Vec3::ZERO, Vec3::ONE, 3.0, 6.0);
    let mut restored_light = PointLight::default();
    restored_light
        .apply(&with_extra_field(light.to_value()))
        .expect("apply must not fail on extra fields");
    assert_eq!(restored_light.to_value(), light.to_value());
}
