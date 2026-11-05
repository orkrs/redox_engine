use redox_math::{Quat, Transform, Vec3};

use crate::meta::{FieldMeta, TypeMeta};
use crate::{Reflect, ReflectError, ReflectField, ReflectSchema, ReflectValue, TypeRegistry};

impl Reflect for Transform {
    fn type_name() -> &'static str {
        "Transform"
    }

    fn reflect_schema() -> ReflectSchema {
        let transform_field = |name: &'static str, type_name: &'static str, meta: FieldMeta| {
            ReflectField {
                name,
                type_name,
                meta,
            }
        };
        ReflectSchema {
            type_name: Self::type_name(),
            fields: vec![
                transform_field(
                    "translation",
                    "Vec3",
                    FieldMeta {
                        category: Some("Transform"),
                        unit: Some("m"),
                        ..Default::default()
                    },
                ),
                transform_field(
                    "rotation",
                    "Quat",
                    FieldMeta {
                        category: Some("Transform"),
                        tooltip: Some("Rotation as a quaternion (x, y, z, w)"),
                        ..Default::default()
                    },
                ),
                transform_field(
                    "scale",
                    "Vec3",
                    FieldMeta {
                        category: Some("Transform"),
                        ..Default::default()
                    },
                ),
            ],
            meta: TypeMeta {
                category: Some("Transform"),
                ..Default::default()
            },
        }
    }

    fn to_value(&self) -> ReflectValue {
        ReflectValue::Struct {
            type_name: Self::type_name().into(),
            fields: vec![
                (
                    "translation".into(),
                    ReflectValue::vec3(self.translation.x, self.translation.y, self.translation.z),
                ),
                (
                    "rotation".into(),
                    ReflectValue::quat(
                        self.rotation.x,
                        self.rotation.y,
                        self.rotation.z,
                        self.rotation.w,
                    ),
                ),
                (
                    "scale".into(),
                    ReflectValue::vec3(self.scale.x, self.scale.y, self.scale.z),
                ),
            ],
        }
    }

    fn apply(&mut self, value: &ReflectValue) -> Result<(), ReflectError> {
        if let Some(v) = value.field("translation").and_then(ReflectValue::as_vec3) {
            self.translation = Vec3::from_array(v);
        }
        if let Some(v) = value.field("rotation").and_then(ReflectValue::as_quat) {
            self.rotation = Quat::from_xyzw(v[0], v[1], v[2], v[3]);
        }
        if let Some(v) = value.field("scale").and_then(ReflectValue::as_vec3) {
            self.scale = Vec3::from_array(v);
        }
        Ok(())
    }
}

/// Registers engine types the editor inspector needs by default.
pub fn register_engine_defaults(registry: &mut TypeRegistry) {
    registry.register::<Transform>();
}
