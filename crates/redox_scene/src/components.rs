//! Components that name and tag scene entities, with their reflected forms.

use redox_reflect::{Reflect, ReflectError, ReflectValue};
use serde::{Deserialize, Serialize};

/// Unique lookup name of an entity (scripts call it with `find_by_name`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Name(pub String);

/// One tag of an entity (scripts search for it with `find_by_tag`).
/// An entity carries one `Tag` component per tag.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Tag(pub String);

impl Reflect for Name {
    fn type_name() -> &'static str {
        "Name"
    }

    fn to_value(&self) -> ReflectValue {
        ReflectValue::Struct {
            type_name: Self::type_name().into(),
            fields: vec![("name".into(), ReflectValue::String(self.0.clone()))],
        }
    }

    fn apply(&mut self, value: &ReflectValue) -> Result<(), ReflectError> {
        if let Some(text) = value.field("name").and_then(ReflectValue::as_string) {
            self.0 = text.to_owned();
        }
        Ok(())
    }
}

impl Reflect for Tag {
    fn type_name() -> &'static str {
        "Tag"
    }

    fn to_value(&self) -> ReflectValue {
        ReflectValue::Struct {
            type_name: Self::type_name().into(),
            fields: vec![("tag".into(), ReflectValue::String(self.0.clone()))],
        }
    }

    fn apply(&mut self, value: &ReflectValue) -> Result<(), ReflectError> {
        if let Some(text) = value.field("tag").and_then(ReflectValue::as_string) {
            self.0 = text.to_owned();
        }
        Ok(())
    }
}
