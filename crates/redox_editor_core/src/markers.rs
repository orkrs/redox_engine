//! Editor markers for scene entities (folders, lock) and visibility for hide/show.

use redox_reflect::{Reflect, ReflectError, ReflectValue};
use serde::{Deserialize, Serialize};

/// Marks an entity as an outliner folder (no runtime logic).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EditorFolder;

/// Marks an entity as locked in the editor (no accidental edits).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Locked;

impl Reflect for EditorFolder {
    fn type_name() -> &'static str {
        "EditorFolder"
    }

    fn to_value(&self) -> ReflectValue {
        ReflectValue::Struct {
            type_name: Self::type_name().into(),
            fields: vec![],
        }
    }

    fn apply(&mut self, _value: &ReflectValue) -> Result<(), ReflectError> {
        Ok(())
    }
}

impl Reflect for Locked {
    fn type_name() -> &'static str {
        "Locked"
    }

    fn to_value(&self) -> ReflectValue {
        ReflectValue::Struct {
            type_name: Self::type_name().into(),
            fields: vec![],
        }
    }

    fn apply(&mut self, _value: &ReflectValue) -> Result<(), ReflectError> {
        Ok(())
    }
}
