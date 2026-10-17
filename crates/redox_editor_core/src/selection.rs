use redox_ecs::Entity;
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
}

/// Component marking whether an entity is shown in the viewport.
#[derive(Clone, Debug)]
pub struct Visibility(pub bool);
impl Default for Visibility {
    fn default() -> Self {
        Self(true)
    }
}
