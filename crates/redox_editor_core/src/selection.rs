use redox_ecs::Entity;
use redox_reflect::{Reflect, ReflectError, ReflectValue};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// The entities selected in the editor.
#[derive(Clone, Debug, Default)]
pub struct Selection {
    /// The entity the gizmo and inspector work on.
    pub primary: Option<Entity>,
    /// All selected entities.
    pub set: HashSet<Entity>,
}

impl Selection {
    /// Makes `e` the primary selection (or clears it) and updates the set.
    pub fn set_primary(&mut self, e: Option<Entity>) {
        self.primary = e;
        self.set.clear();
        if let Some(ent) = e {
            self.set.insert(ent);
        }
    }
    /// Adds `e` to the selection, or removes it when already selected.
    pub fn toggle(&mut self, e: Entity) {
        if !self.set.insert(e) {
            self.set.remove(&e);
        }
        self.primary = self.set.iter().next().copied();
    }
    /// Clears the selection.
    pub fn clear(&mut self) {
        self.primary = None;
        self.set.clear();
    }

    /// Adds `entity` to the selection and makes it primary.
    pub fn add(&mut self, entity: Entity) {
        self.set.insert(entity);
        self.primary = Some(entity);
    }

    /// Replaces the selection with every entity in `entities` (empty clears).
    pub fn replace(&mut self, entities: impl IntoIterator<Item = Entity>) {
        self.set.clear();
        self.primary = None;
        for entity in entities {
            self.set.insert(entity);
            if self.primary.is_none() {
                self.primary = Some(entity);
            }
        }
    }
}

/// Component marking whether an entity is shown in the viewport.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Visibility(pub bool);

impl Default for Visibility {
    fn default() -> Self {
        Self(true)
    }
}

impl Reflect for Visibility {
    fn type_name() -> &'static str {
        "Visibility"
    }

    fn to_value(&self) -> ReflectValue {
        ReflectValue::Struct {
            type_name: Self::type_name().into(),
            fields: vec![("visible".into(), ReflectValue::Bool(self.0))],
        }
    }

    fn apply(&mut self, value: &ReflectValue) -> Result<(), ReflectError> {
        if let Some(visible) = value.field("visible").and_then(ReflectValue::as_bool) {
            self.0 = visible;
        }
        Ok(())
    }
}
