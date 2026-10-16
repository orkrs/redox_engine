use redox_ecs::{Entity, World};
use redox_reflect::{EntitySnapshot, ReflectValue, TypeRegistry, WorldSnapshot};

#[derive(Clone, Debug)]
pub enum UndoCommand {
    SetComponent {
        entity: Entity,
        type_name: String,
        before: Option<ReflectValue>,
        after: Option<ReflectValue>,
    },
    Spawn {
        entity: Entity,
        snapshot: EntitySnapshot,
    },
    Despawn {
        entity: Entity,
        snapshot: EntitySnapshot,
    },
    Batch(Vec<UndoCommand>),
}

impl UndoCommand {
    pub fn apply(&self, world: &mut World, registry: &TypeRegistry) {
        match self {
            UndoCommand::SetComponent {
                entity,
                type_name,
                after,
                ..
            } => {
                if let Some(val) = after {
                    let _ = registry.insert_component(world, *entity, val);
                } else {
                    let _ = type_name;
                }
            }
            UndoCommand::Spawn {
                entity: _,
                snapshot,
            } => {
                let e = world.spawn();
                for comp in &snapshot.components {
                    let _ = registry.insert_component(world, e, comp);
                }
            }
            UndoCommand::Despawn { entity, .. } => {
                world.despawn(*entity);
            }
            UndoCommand::Batch(cmds) => {
                for c in cmds {
                    c.apply(world, registry);
                }
            }
        }
    }
    pub fn invert(&self) -> Self {
        match self {
            UndoCommand::SetComponent {
                entity,
                type_name,
                before,
                after,
            } => UndoCommand::SetComponent {
                entity: *entity,
                type_name: type_name.clone(),
                before: after.clone(),
                after: before.clone(),
            },
            UndoCommand::Spawn { entity, snapshot } => UndoCommand::Despawn {
                entity: *entity,
                snapshot: snapshot.clone(),
            },
            UndoCommand::Despawn { entity, snapshot } => UndoCommand::Spawn {
                entity: *entity,
                snapshot: snapshot.clone(),
            },
            UndoCommand::Batch(cmds) => {
                UndoCommand::Batch(cmds.iter().map(|c| c.invert()).collect())
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct UndoStack {
    pub undo: Vec<UndoCommand>,
    pub redo: Vec<UndoCommand>,
    pub limit: usize,
    pub last_push_time: std::time::Instant,
    pub merge_window_secs: f32,
}

impl Default for UndoStack {
    fn default() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            limit: 100,
            last_push_time: std::time::Instant::now(),
            merge_window_secs: 0.5,
        }
    }
}

impl UndoStack {
    pub fn new(limit: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            limit,
            last_push_time: std::time::Instant::now(),
            merge_window_secs: 0.5,
        }
    }
    pub fn push(&mut self, cmd: UndoCommand) {
        // Merge sequential SetComponent on same entity/type within window
        if let UndoCommand::SetComponent {
            entity,
            type_name,
            before: _,
            after,
        } = &cmd
            && let Some(UndoCommand::SetComponent {
                entity: e2,
                type_name: t2,
                before: b2,
                after: _,
            }) = self.undo.last()
            && e2 == entity
            && t2 == type_name
            && self.last_push_time.elapsed().as_secs_f32() < self.merge_window_secs
        {
            // Merge: keep original before, update after
            let merged = UndoCommand::SetComponent {
                entity: *entity,
                type_name: type_name.clone(),
                before: b2.clone(),
                after: after.clone(),
            };
            self.undo.pop();
            self.undo.push(merged);
            self.last_push_time = std::time::Instant::now();
            self.redo.clear();
            return;
        }
        self.undo.push(cmd);
        self.redo.clear();
        self.last_push_time = std::time::Instant::now();
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo(&mut self, world: &mut World, registry: &TypeRegistry) -> bool {
        if let Some(cmd) = self.undo.pop() {
            let inv = cmd.invert();
            inv.apply(world, registry);
            self.redo.push(cmd);
            true
        } else {
            false
        }
    }
    pub fn redo(&mut self, world: &mut World, registry: &TypeRegistry) -> bool {
        if let Some(cmd) = self.redo.pop() {
            // cmd is original, need to apply it (not invert)
            // To redo, we apply the original command again
            // But we stored original in redo, so we need to re-apply original
            // We have cmd = original that was undone, so apply it
            let redo_cmd = cmd.clone();
            redo_cmd.apply(world, registry);
            self.undo.push(redo_cmd);
            true
        } else {
            false
        }
    }

    /// Handle Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z via egui input
    pub fn handle_hotkeys(
        &mut self,
        ctx: &egui::Context,
        world: &mut World,
        registry: &TypeRegistry,
    ) -> bool {
        let ctrl_z =
            ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Z) && !i.modifiers.shift);
        let ctrl_y = ctx.input(|i| {
            i.modifiers.ctrl
                && (i.key_pressed(egui::Key::Y)
                    || (i.key_pressed(egui::Key::Z) && i.modifiers.shift))
        });
        if ctrl_z {
            return self.undo(world, registry);
        }
        if ctrl_y {
            return self.redo(world, registry);
        }
        false
    }
}

pub fn capture_entity(world: &World, entity: Entity, registry: &TypeRegistry) -> EntitySnapshot {
    EntitySnapshot {
        components: registry.snapshot_entity(world, entity),
    }
}
pub fn capture_world(world: &World, registry: &TypeRegistry) -> WorldSnapshot {
    registry.snapshot_world(world)
}
pub fn restore_world(world: &mut World, snapshot: &WorldSnapshot, registry: &TypeRegistry) {
    let entities: Vec<_> = world.all_entities().collect();
    for e in entities {
        world.despawn(e);
    }
    for ent_snap in &snapshot.entities {
        let e = world.spawn();
        for comp in &ent_snap.components {
            let _ = registry.insert_component(world, e, comp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_ecs::World;
    use redox_math::{Transform, Vec3};
    use redox_reflect::{Reflect, TypeRegistry};
    #[test]
    fn undo_redo_transform() {
        let mut world = World::new();
        let mut registry = TypeRegistry::new();
        registry.register::<Transform>();
        let e = world.spawn();
        world.add_component(e, Transform::from_translation(Vec3::ZERO));
        let mut undo = UndoStack::default();
        let before = Transform::from_translation(Vec3::ZERO).to_value();
        let after = Transform::from_translation(Vec3::new(2.0, 0.0, 0.0)).to_value();
        undo.push(UndoCommand::SetComponent {
            entity: e,
            type_name: "Transform".into(),
            before: Some(before.clone()),
            after: Some(after.clone()),
        });
        // Apply after
        registry.insert_component(&mut world, e, &after).unwrap();
        assert_eq!(
            world.get_component::<Transform>(e).unwrap().translation.x,
            2.0
        );
        assert!(undo.undo(&mut world, &registry));
        assert_eq!(
            world.get_component::<Transform>(e).unwrap().translation.x,
            0.0
        );
        assert!(undo.redo(&mut world, &registry));
        assert_eq!(
            world.get_component::<Transform>(e).unwrap().translation.x,
            2.0
        );
    }
    #[test]
    fn capture_restore_world() {
        let mut world = World::new();
        let mut registry = TypeRegistry::new();
        registry.register::<Transform>();
        let e = world.spawn();
        world.add_component(e, Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)));
        let snap = capture_world(&world, &registry);
        // Modify
        world.get_component_mut::<Transform>(e).unwrap().translation = Vec3::ZERO;
        assert_eq!(
            world.get_component::<Transform>(e).unwrap().translation,
            Vec3::ZERO
        );
        restore_world(&mut world, &snap, &registry);
        // After restore, should have one entity with original transform (id may differ)
        let count = world.all_entities().count();
        assert_eq!(count, 1);
        let e2 = world.all_entities().next().unwrap();
        assert_eq!(
            world.get_component::<Transform>(e2).unwrap().translation,
            Vec3::new(1.0, 2.0, 3.0)
        );
    }
}
