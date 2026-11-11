//! Components that name and tag scene entities, the stable entity identity and the parent link
//! in its saved form, with their reflected values.

use std::fmt;
use std::str::FromStr;

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

/// Stable identity of a scene entity: 128 random bits, written as 32 lowercase hex digits.
///
/// The id is kept by save and load, so references between entities (the hierarchy) survive a
/// restart. An entity duplicated by the editor gets a new id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityGuid(pub u128);

impl EntityGuid {
    /// Generates a random (version 4) id.
    pub fn new_random() -> Self {
        Self(uuid::Uuid::new_v4().as_u128())
    }
}

impl fmt::Display for EntityGuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

/// Text that is not 32 hexadecimal digits cannot be an [`EntityGuid`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseEntityGuidError(String);

impl fmt::Display for ParseEntityGuidError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid entity guid {:?}: expected 32 hexadecimal digits",
            self.0
        )
    }
}

impl std::error::Error for ParseEntityGuidError {}

impl FromStr for EntityGuid {
    type Err = ParseEntityGuidError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ParseEntityGuidError(s.to_owned()));
        }
        u128::from_str_radix(s, 16)
            .map(Self)
            .map_err(|_| ParseEntityGuidError(s.to_owned()))
    }
}

impl Serialize for EntityGuid {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for EntityGuid {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse::<Self>()
            .map_err(<D::Error as serde::de::Error>::custom)
    }
}

impl Reflect for EntityGuid {
    fn type_name() -> &'static str {
        "EntityGuid"
    }

    fn to_value(&self) -> ReflectValue {
        ReflectValue::Struct {
            type_name: Self::type_name().into(),
            fields: vec![("value".into(), ReflectValue::String(self.to_string()))],
        }
    }

    fn apply(&mut self, value: &ReflectValue) -> Result<(), ReflectError> {
        if let Some(parsed) = value
            .field("value")
            .and_then(ReflectValue::as_string)
            .and_then(|text| text.parse::<EntityGuid>().ok())
        {
            *self = parsed;
        }
        Ok(())
    }
}

/// The saved form of a parent link: the parent's [`EntityGuid`].
///
/// `Parent` (in `redox_ecs`) holds an entity handle, which means nothing in a file. The scene
/// writer stores a `ParentRef` instead, and the loader turns it back into `set_parent` after
/// every entity exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParentRef(pub EntityGuid);

impl ParentRef {
    /// Reads a parent link from a reflected value; `None` when the value has no valid `parent`.
    pub fn from_value(value: &ReflectValue) -> Option<Self> {
        value
            .field("parent")
            .and_then(ReflectValue::as_string)
            .and_then(|text| text.parse::<EntityGuid>().ok())
            .map(Self)
    }
}

impl Reflect for ParentRef {
    fn type_name() -> &'static str {
        "ParentRef"
    }

    fn to_value(&self) -> ReflectValue {
        ReflectValue::Struct {
            type_name: Self::type_name().into(),
            fields: vec![("parent".into(), ReflectValue::String(self.0.to_string()))],
        }
    }

    fn apply(&mut self, value: &ReflectValue) -> Result<(), ReflectError> {
        if let Some(parent) = Self::from_value(value) {
            *self = parent;
        }
        Ok(())
    }
}

/// One saved asset reference of a component: the GUID is the source of truth, the path a hint.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssetRef {
    /// Reflected type name of the handle component (`MeshHandle`, `MaterialHandle`).
    pub component: String,
    /// Asset GUID as 32 hex digits. `None` when the reference is unset.
    pub guid: Option<String>,
    /// Path hint relative to the assets root. `None` when unset.
    pub path: Option<String>,
}

/// The asset references of an entity, kept in the world next to the render handles.
///
/// A render handle knows only a runtime slot, so the GUID and path read from a scene file would
/// be lost. This component keeps them; the scene writer puts them back into the handle values.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssetRefs(pub Vec<AssetRef>);

impl AssetRefs {
    /// The reference stored for `component`.
    pub fn get(&self, component: &str) -> Option<&AssetRef> {
        self.0.iter().find(|r| r.component == component)
    }

    /// Stores a reference for `component`, replacing an earlier one.
    pub fn set(&mut self, component: &str, guid: Option<String>, path: Option<String>) {
        let entry = AssetRef {
            component: component.to_owned(),
            guid,
            path,
        };
        match self.0.iter_mut().find(|r| r.component == component) {
            Some(existing) => *existing = entry,
            None => self.0.push(entry),
        }
    }
}

impl Reflect for AssetRefs {
    fn type_name() -> &'static str {
        "AssetRefs"
    }

    fn to_value(&self) -> ReflectValue {
        let list = self
            .0
            .iter()
            .map(|r| ReflectValue::asset_ref(r.component.clone(), r.guid.clone(), r.path.clone()))
            .collect();
        ReflectValue::Struct {
            type_name: Self::type_name().into(),
            fields: vec![("refs".into(), ReflectValue::List(list))],
        }
    }

    fn apply(&mut self, value: &ReflectValue) -> Result<(), ReflectError> {
        let Some(ReflectValue::List(items)) = value.field("refs") else {
            return Ok(());
        };
        self.0 = items
            .iter()
            .filter_map(|item| {
                let ReflectValue::Struct { type_name, .. } = item else {
                    return None;
                };
                Some(AssetRef {
                    component: type_name.clone(),
                    guid: item
                        .field("guid")
                        .and_then(ReflectValue::as_string)
                        .map(str::to_owned),
                    path: item.as_handle_path().flatten(),
                })
            })
            .collect();
        Ok(())
    }
}
