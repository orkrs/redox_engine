use redox_ecs::Entity;
use std::collections::HashSet;

#[derive(Clone, Debug, Default)]
pub struct Selection {
    pub primary: Option<Entity>,
    pub set: HashSet<Entity>,
}

impl Selection {
    pub fn set_primary(&mut self, e: Option<Entity>) {
        self.primary = e;
        self.set.clear();
        if let Some(ent) = e {
            self.set.insert(ent);
        }
    }
    pub fn toggle(&mut self, e: Entity) {
        if !self.set.insert(e) {
            self.set.remove(&e);
        }
        self.primary = self.set.iter().next().copied();
    }
    pub fn clear(&mut self) {
        self.primary = None;
        self.set.clear();
    }
}

#[derive(Clone, Debug)]
pub struct Visibility(pub bool);
impl Default for Visibility {
    fn default() -> Self {
        Self(true)
    }
}
