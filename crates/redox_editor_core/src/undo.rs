use std::fmt;

use redox_ecs::{Entity, World};
use redox_reflect::{EntitySnapshot, ReflectValue, TypeRegistry, WorldSnapshot};
use redox_scene::components::EntityGuid;

use crate::guid::GuidResolver;

/// Why an undo command could not be applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UndoError {
    /// No entity with this GUID exists in the world.
    EntityNotFound(EntityGuid),
    /// The component type is not registered.
    UnknownType(String),
    /// The reflected value could not be written to the world.
    Apply(String),
}

impl fmt::Display for UndoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EntityNotFound(guid) => write!(f, "entity {guid:?} not found"),
            Self::UnknownType(name) => write!(f, "unknown component type {name}"),
            Self::Apply(message) => write!(f, "could not apply value: {message}"),
        }
    }
}

impl std::error::Error for UndoError {}

/// A reversible editor action. Entities are addressed by [`EntityGuid`], so a command stays valid
/// after its entity was deleted and restored (the handle changes, the GUID does not).
#[derive(Clone, Debug)]
pub enum UndoCommand {
    /// Changed one component of an entity.
    SetComponent {
        /// The entity that was edited.
        guid: EntityGuid,
        /// Registered name of the component type.
        type_name: String,
        /// Value before the edit (`None`: the component did not exist).
        before: Option<ReflectValue>,
        /// Value after the edit (`None`: the component is removed).
        after: Option<ReflectValue>,
    },
    /// Created an entity.
    Spawn {
        /// GUID given to the recreated entity.
        guid: EntityGuid,
        /// Components of the entity, used to recreate it on redo.
        snapshot: EntitySnapshot,
    },
    /// Deleted an entity.
    Despawn {
        /// The entity that was deleted.
        guid: EntityGuid,
        /// Components of the entity, used to bring it back on undo.
        snapshot: EntitySnapshot,
    },
    /// Several commands applied and undone as one step.
    Batch(Vec<UndoCommand>),
}

impl UndoCommand {
    /// Applies the command to `world`.
    pub fn apply(&self, world: &mut World, registry: &TypeRegistry) -> Result<(), UndoError> {
        match self {
            Self::SetComponent {
                guid,
                type_name,
                after,
                ..
            } => {
                let entity = resolve(world, *guid)?;
                set_component(world, registry, entity, type_name, after.as_ref())
            }
            Self::Spawn { guid, snapshot } => {
                let entity = world.spawn();
                for component in &snapshot.components {
                    insert_value(world, registry, entity, component)?;
                }
                world.add_component(entity, *guid);
                Ok(())
            }
            Self::Despawn { guid, .. } => {
                let entity = resolve(world, *guid)?;
                world.despawn(entity);
                Ok(())
            }
            Self::Batch(commands) => commands
                .iter()
                .try_for_each(|command| command.apply(world, registry)),
        }
    }

    /// The command that reverses this one.
    pub fn invert(&self) -> Self {
        match self {
            Self::SetComponent {
                guid,
                type_name,
                before,
                after,
            } => Self::SetComponent {
                guid: *guid,
                type_name: type_name.clone(),
                before: after.clone(),
                after: before.clone(),
            },
            Self::Spawn { guid, snapshot } => Self::Despawn {
                guid: *guid,
                snapshot: snapshot.clone(),
            },
            Self::Despawn { guid, snapshot } => Self::Spawn {
                guid: *guid,
                snapshot: snapshot.clone(),
            },
            Self::Batch(commands) => Self::Batch(commands.iter().map(Self::invert).collect()),
        }
    }
}

fn resolve(world: &World, guid: EntityGuid) -> Result<Entity, UndoError> {
    GuidResolver::resolve(world, guid).ok_or(UndoError::EntityNotFound(guid))
}

fn insert_value(
    world: &mut World,
    registry: &TypeRegistry,
    entity: Entity,
    value: &ReflectValue,
) -> Result<(), UndoError> {
    if let ReflectValue::Struct { type_name, .. } = value
        && registry.schema(type_name).is_none()
    {
        return Err(UndoError::UnknownType(type_name.clone()));
    }
    registry
        .insert_component(world, entity, value)
        .map_err(|error| UndoError::Apply(error.to_string()))
}

fn set_component(
    world: &mut World,
    registry: &TypeRegistry,
    entity: Entity,
    type_name: &str,
    after: Option<&ReflectValue>,
) -> Result<(), UndoError> {
    if registry.schema(type_name).is_none() {
        return Err(UndoError::UnknownType(type_name.to_string()));
    }
    match after {
        Some(value) => insert_value(world, registry, entity, value),
        None => {
            registry.remove_component(world, entity, type_name);
            Ok(())
        }
    }
}

/// Undo and redo history with merging of rapid edits.
#[derive(Clone, Debug)]
pub struct UndoStack {
    /// Commands that can be undone, newest last.
    pub undo: Vec<UndoCommand>,
    /// Commands that can be redone, newest last.
    pub redo: Vec<UndoCommand>,
    /// Maximum number of undo steps kept.
    pub limit: usize,
    /// When the last command was pushed.
    pub last_push_time: std::time::Instant,
    /// Edits of the same value closer than this many seconds merge into one step.
    pub merge_window_secs: f32,
    /// Error of the last undo or redo that could not be applied.
    pub last_error: Option<UndoError>,
}

impl Default for UndoStack {
    fn default() -> Self {
        Self::new(100)
    }
}

impl UndoStack {
    /// Creates a stack that keeps up to `limit` steps.
    pub fn new(limit: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            limit,
            last_push_time: std::time::Instant::now(),
            merge_window_secs: 0.5,
            last_error: None,
        }
    }

    /// Records a command that was already applied; it may merge with the previous one, and clears
    /// the redo history.
    pub fn push(&mut self, cmd: UndoCommand) {
        if self.merge_into_last(&cmd) {
            self.redo.clear();
            self.last_push_time = std::time::Instant::now();
            return;
        }
        self.undo.push(cmd);
        self.redo.clear();
        self.last_push_time = std::time::Instant::now();
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
    }

    /// Replaces the last command with a merged one when both edit the same value of the same
    /// entity within the merge window. The merged command keeps the original `before`.
    fn merge_into_last(&mut self, cmd: &UndoCommand) -> bool {
        let UndoCommand::SetComponent {
            guid,
            type_name,
            after,
            ..
        } = cmd
        else {
            return false;
        };
        if self.last_push_time.elapsed().as_secs_f32() >= self.merge_window_secs {
            return false;
        }
        let merged = match self.undo.last() {
            Some(UndoCommand::SetComponent {
                guid: last_guid,
                type_name: last_type,
                before: last_before,
                ..
            }) if last_guid == guid && last_type == type_name => UndoCommand::SetComponent {
                guid: *guid,
                type_name: type_name.clone(),
                before: last_before.clone(),
                after: after.clone(),
            },
            _ => return false,
        };
        if let Some(slot) = self.undo.last_mut() {
            *slot = merged;
        }
        true
    }

    /// Whether there is something to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether there is something to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Undoes the last command; returns whether anything was undone. A command that cannot be
    /// applied stays in the history and its error is stored in [`UndoStack::last_error`].
    pub fn undo(&mut self, world: &mut World, registry: &TypeRegistry) -> bool {
        let Some(inverse) = self.undo.last().map(UndoCommand::invert) else {
            return false;
        };
        if let Err(error) = inverse.apply(world, registry) {
            self.last_error = Some(error);
            return false;
        }
        if let Some(cmd) = self.undo.pop() {
            self.redo.push(cmd);
        }
        true
    }

    /// Redoes the last undone command; returns whether anything was redone. A command that cannot
    /// be applied stays in the redo history and its error is stored in [`UndoStack::last_error`].
    pub fn redo(&mut self, world: &mut World, registry: &TypeRegistry) -> bool {
        let Some(cmd) = self.redo.last() else {
            return false;
        };
        if let Err(error) = cmd.apply(world, registry) {
            self.last_error = Some(error);
            return false;
        }
        if let Some(cmd) = self.redo.pop() {
            self.undo.push(cmd);
        }
        true
    }

    /// Applies an undo or redo requested by a keyboard shortcut; returns whether anything changed.
    pub fn apply_hotkey(
        &mut self,
        hotkey: UndoHotkey,
        world: &mut World,
        registry: &TypeRegistry,
    ) -> bool {
        match hotkey {
            UndoHotkey::Undo => self.undo(world, registry),
            UndoHotkey::Redo => self.redo(world, registry),
        }
    }
}

/// An undo shortcut recognised by the UI (Ctrl+Z, Ctrl+Y or Ctrl+Shift+Z).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UndoHotkey {
    /// Undo the last step.
    Undo,
    /// Redo the last undone step.
    Redo,
}

/// Captures all registered components of an entity.
pub fn capture_entity(world: &World, entity: Entity, registry: &TypeRegistry) -> EntitySnapshot {
    EntitySnapshot {
        components: registry.snapshot_entity(world, entity),
    }
}
/// Captures all entities with registered components.
pub fn capture_world(world: &World, registry: &TypeRegistry) -> WorldSnapshot {
    registry.snapshot_world(world)
}
/// Replaces the world contents with `snapshot`.
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
        let guid = EntityGuid(1);
        world.add_component(e, guid);
        world.add_component(e, Transform::from_translation(Vec3::ZERO));
        let mut undo = UndoStack::default();
        let before = Transform::from_translation(Vec3::ZERO).to_value();
        let after = Transform::from_translation(Vec3::new(2.0, 0.0, 0.0)).to_value();
        undo.push(UndoCommand::SetComponent {
            guid,
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
