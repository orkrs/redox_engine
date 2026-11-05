//! Editor metadata of reflected types and fields: ranges, labels, defaults and flags.
//!
//! Metadata is written with `#[reflect(...)]` attributes on `#[derive(Reflect)]` and read through
//! [`crate::ReflectSchema`]. It never changes how a component is saved or simulated.

use std::fmt;

use crate::{ReflectField, ReflectValue};

/// How the inspector draws a field. `Auto` picks a widget from the shape of the value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WidgetHint {
    /// Chosen from the shape of the value.
    #[default]
    Auto,
    /// Three numbers shown as a colour picker.
    Color,
    /// Radians, shown and typed in degrees.
    Angle,
    /// Number with a slider over the field range.
    Slider,
    /// Multi-line text.
    Multiline,
    /// File path with a picker.
    Path,
    /// Text chosen from a fixed list of options.
    Enum(&'static [&'static str]),
}

/// Editor metadata of one field. The default value means "no metadata".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FieldMeta {
    /// Label shown instead of the field name.
    pub label: Option<&'static str>,
    /// Group the field is listed under in the inspector.
    pub category: Option<&'static str>,
    /// Hint shown when the pointer rests on the field.
    pub tooltip: Option<&'static str>,
    /// Smallest and largest allowed number.
    pub range: Option<(f64, f64)>,
    /// Grid that numbers are rounded to.
    pub step: Option<f64>,
    /// Unit shown after the value, e.g. `m`.
    pub unit: Option<&'static str>,
    /// Value used by Reset; the field's `Default` is used when `None`.
    pub default: Option<ReflectValue>,
    /// The field can be read but not written.
    pub read_only: bool,
    /// The field is not shown in the inspector.
    pub hidden: bool,
    /// The field is shown only when advanced fields are enabled.
    pub advanced: bool,
    /// The field is for the editor only and is never shown in the inspector.
    pub editor_only: bool,
    /// Widget to draw the field with.
    pub widget: WidgetHint,
}

/// Editor metadata of a whole type.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeMeta {
    /// Group the component is listed under in the add menu.
    pub category: Option<&'static str>,
    /// Hint shown for the component.
    pub tooltip: Option<&'static str>,
    /// Component types this one needs on the same entity.
    pub requires: &'static [&'static str],
    /// Schema version of the type. Old values are upgraded with `Reflect::migrate`.
    pub version: u32,
    /// The component is for the editor only.
    pub editor_only: bool,
}

impl Default for TypeMeta {
    fn default() -> Self {
        Self {
            category: None,
            tooltip: None,
            requires: &[],
            version: 1,
            editor_only: false,
        }
    }
}

/// Error from validating or writing a field value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldError {
    /// The field is marked `read_only`.
    ReadOnly,
    /// The value does not have the shape the field type needs. Holds the expected type name.
    TypeMismatch {
        /// Name of the type the field expects.
        expected: String,
    },
    /// No component type with this name is registered.
    UnknownType(String),
    /// The component type has no field with this name.
    UnknownField(String),
    /// The entity does not have the component.
    MissingComponent(String),
    /// Any other failure while applying the value.
    Reflect(String),
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadOnly => write!(f, "field is read-only"),
            Self::TypeMismatch { expected } => {
                write!(f, "value does not match the field type {expected}")
            }
            Self::UnknownType(name) => write!(f, "unknown component type {name}"),
            Self::UnknownField(name) => write!(f, "unknown field {name}"),
            Self::MissingComponent(name) => write!(f, "entity has no {name} component"),
            Self::Reflect(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for FieldError {}

impl FieldMeta {
    /// Checks a value against the metadata: read-only fields refuse writes, numbers are rounded
    /// to `step` and clamped to `range`. Values that are not numbers pass unchanged.
    pub fn validate(&self, value: &ReflectValue) -> Result<ReflectValue, FieldError> {
        if self.read_only {
            return Err(FieldError::ReadOnly);
        }
        let Some(number) = as_number(value) else {
            return Ok(value.clone());
        };
        let stepped = match self.step {
            Some(step) if step > 0.0 => (number / step).round() * step,
            _ => number,
        };
        let clamped = match self.range {
            Some((min, max)) => stepped.max(min).min(max),
            None => stepped,
        };
        Ok(match value {
            ReflectValue::I64(_) => ReflectValue::I64(clamped.round() as i64),
            _ => ReflectValue::F64(clamped),
        })
    }
}

impl ReflectField {
    /// Checks a value for this field: the read-only flag, the shape for the field type, then the
    /// metadata rules. Returns the value to store.
    pub fn validate(&self, value: &ReflectValue) -> Result<ReflectValue, FieldError> {
        if self.meta.read_only {
            return Err(FieldError::ReadOnly);
        }
        check_type(self.type_name, value)?;
        self.meta.validate(value)
    }
}

/// Checks that `value` has the shape the field type `type_name` needs.
///
/// Types that are not known here (custom types, asset handles) are accepted as they are.
pub fn check_type(type_name: &str, value: &ReflectValue) -> Result<(), FieldError> {
    let fits = match type_name {
        "f32" | "f64" => as_number(value).is_some(),
        "i8" | "i16" | "i32" | "i64" | "isize" | "u8" | "u16" | "u32" | "u64" | "usize" => {
            matches!(value, ReflectValue::I64(_))
        }
        "bool" => matches!(value, ReflectValue::Bool(_)),
        "String" => matches!(value, ReflectValue::String(_)),
        "Vec2" => matches!(
            value,
            ReflectValue::List(items)
                if items.len() == 2 && items.iter().all(|v| as_number(v).is_some())
        ),
        "Vec3" => value.as_vec3().is_some(),
        "Quat" => value.as_quat().is_some(),
        _ => true,
    };
    if fits {
        Ok(())
    } else {
        Err(FieldError::TypeMismatch {
            expected: type_name.to_string(),
        })
    }
}

fn as_number(value: &ReflectValue) -> Option<f64> {
    match value {
        ReflectValue::F64(v) => Some(*v),
        ReflectValue::I64(v) => Some(*v as f64),
        _ => None,
    }
}
