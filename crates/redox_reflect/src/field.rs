//! Conversion of single field values, used by `#[derive(Reflect)]`.

use redox_math::{Quat, Vec2, Vec3};

use crate::ReflectValue;

/// A type that can be one field of a reflected struct.
///
/// `apply_field_value` leaves the field unchanged when the value has the wrong shape
/// (for example a string for a number), so partially valid input never corrupts a field.
pub trait FieldValue {
    /// Name shown by inspectors.
    const TYPE_NAME: &'static str;
    /// Converts the field to its reflected form.
    fn to_field_value(&self) -> ReflectValue;
    /// Overwrites the field from a reflected form, ignoring values of the wrong shape.
    fn apply_field_value(&mut self, value: &ReflectValue);
}

macro_rules! float_field {
    ($($t:ty),*) => {$(
        impl FieldValue for $t {
            const TYPE_NAME: &'static str = stringify!($t);
            fn to_field_value(&self) -> ReflectValue {
                ReflectValue::F64(f64::from(*self))
            }
            fn apply_field_value(&mut self, value: &ReflectValue) {
                match value {
                    ReflectValue::F64(v) => *self = *v as $t,
                    ReflectValue::I64(v) => *self = *v as $t,
                    _ => {}
                }
            }
        }
    )*};
}
float_field!(f32, f64);

macro_rules! int_field {
    ($($t:ty),*) => {$(
        impl FieldValue for $t {
            const TYPE_NAME: &'static str = stringify!($t);
            fn to_field_value(&self) -> ReflectValue {
                ReflectValue::I64(*self as i64)
            }
            #[allow(irrefutable_let_patterns)] // `i64 -> i64` cannot fail
            fn apply_field_value(&mut self, value: &ReflectValue) {
                if let ReflectValue::I64(v) = value {
                    if let Ok(v) = <$t>::try_from(*v) {
                        *self = v;
                    }
                }
            }
        }
    )*};
}
int_field!(i8, i16, i32, i64, isize, u8, u16, u32, usize);

impl FieldValue for u64 {
    const TYPE_NAME: &'static str = "u64";
    fn to_field_value(&self) -> ReflectValue {
        // Values above i64::MAX cannot be represented and are saturated.
        ReflectValue::I64(i64::try_from(*self).unwrap_or(i64::MAX))
    }
    fn apply_field_value(&mut self, value: &ReflectValue) {
        if let ReflectValue::I64(v) = value {
            if let Ok(v) = u64::try_from(*v) {
                *self = v;
            }
        }
    }
}

impl FieldValue for bool {
    const TYPE_NAME: &'static str = "bool";
    fn to_field_value(&self) -> ReflectValue {
        ReflectValue::Bool(*self)
    }
    fn apply_field_value(&mut self, value: &ReflectValue) {
        if let ReflectValue::Bool(v) = value {
            *self = *v;
        }
    }
}

impl FieldValue for String {
    const TYPE_NAME: &'static str = "String";
    fn to_field_value(&self) -> ReflectValue {
        ReflectValue::String(self.clone())
    }
    fn apply_field_value(&mut self, value: &ReflectValue) {
        if let ReflectValue::String(v) = value {
            self.clone_from(v);
        }
    }
}

impl FieldValue for Vec3 {
    const TYPE_NAME: &'static str = "Vec3";
    fn to_field_value(&self) -> ReflectValue {
        ReflectValue::vec3(self.x, self.y, self.z)
    }
    fn apply_field_value(&mut self, value: &ReflectValue) {
        if let Some(v) = value.as_vec3() {
            *self = Vec3::from_array(v);
        }
    }
}

impl FieldValue for Vec2 {
    const TYPE_NAME: &'static str = "Vec2";
    fn to_field_value(&self) -> ReflectValue {
        ReflectValue::List(vec![ReflectValue::f32(self.x), ReflectValue::f32(self.y)])
    }
    fn apply_field_value(&mut self, value: &ReflectValue) {
        if let ReflectValue::List(items) = value {
            if let [x, y] = items.as_slice() {
                if let (Some(x), Some(y)) = (x.as_f32(), y.as_f32()) {
                    *self = Vec2::new(x, y);
                }
            }
        }
    }
}

impl FieldValue for Quat {
    const TYPE_NAME: &'static str = "Quat";
    fn to_field_value(&self) -> ReflectValue {
        ReflectValue::quat(self.x, self.y, self.z, self.w)
    }
    fn apply_field_value(&mut self, value: &ReflectValue) {
        if let Some(q) = value.as_quat() {
            *self = Quat::from_xyzw(q[0], q[1], q[2], q[3]);
        }
    }
}
