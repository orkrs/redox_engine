use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use redox_ecs::hierarchy::{Children, Parent, set_parent};
use redox_ecs::{Entity, World};
use redox_math::{Mat4, Transform};
use redox_reflect::{EntitySnapshot, ReflectValue, TypeRegistry, WorldSnapshot};
use redox_scene::components::{EntityGuid, Name};

use crate::guid::GuidResolver;

/// Default number of undo steps kept.
const DEFAULT_LIMIT: usize = 100;
/// Default window in seconds in which edits of one value merge into one step.
const DEFAULT_MERGE_WINDOW_SECS: f32 = 0.5;

/// Source of the time used for merging edits. The editor uses [`SystemClock`]; tests use a
/// manual clock so that the merge window does not depend on real time.
pub trait Clock: Send + Sync {
    /// The current instant.
    fn now(&self) -> Instant;
}

/// Clock that reads the operating system time.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Why an undo command could not be applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UndoError {
    /// No entity with this GUID exists in the world.
    EntityNotFound(EntityGuid),
    /// The component type is not registered.
    UnknownType(String),
    /// The reflected value could not be written to the world.
    Apply(String),
    /// The new parent is the entity itself or one of its descendants.
    Cycle,
}

impl fmt::Display for UndoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EntityNotFound(guid) => write!(f, "entity {guid:?} not found"),
            Self::UnknownType(name) => write!(f, "unknown component type {name}"),
            Self::Apply(message) => write!(f, "could not apply value: {message}"),
            Self::Cycle => write!(f, "an entity cannot become its own ancestor"),
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
    /// Changed the parent of an entity and its local transform.
    Reparent {
        /// The entity that was moved.
        child: EntityGuid,
        /// Parent before the move (`None`: the entity was a root).
        old_parent: Option<EntityGuid>,
        /// Parent after the move (`None`: the entity becomes a root).
        new_parent: Option<EntityGuid>,
        /// Whether the local transform was recomputed to keep the world position.
        keep_world: bool,
        /// Local transform before the move.
        old_local: Transform,
        /// Local transform after the move.
        new_local: Transform,
    },
    /// Changed the [`Name`] of an entity (`None` removes the component).
    Rename {
        /// The renamed entity.
        guid: EntityGuid,
        /// Name before the change.
        before: Option<String>,
        /// Name after the change.
        after: Option<String>,
    },
}

impl UndoCommand {
    /// Adds a component of a registered type (an undoable `SetComponent` from nothing).
    pub fn add_component(
        guid: EntityGuid,
        type_name: impl Into<String>,
        value: ReflectValue,
    ) -> Self {
        Self::SetComponent {
            guid,
            type_name: type_name.into(),
            before: None,
            after: Some(value),
        }
    }

    /// Removes a component of a registered type; `before` is its value, restored by undo.
    pub fn remove_component(
        guid: EntityGuid,
        type_name: impl Into<String>,
        before: ReflectValue,
    ) -> Self {
        Self::SetComponent {
            guid,
            type_name: type_name.into(),
            before: Some(before),
            after: None,
        }
    }

    /// Builds a move of `child` under `new_parent` (`None` makes it a root). With `keep_world` the
    /// local transform is recomputed so the entity stays where it is in the world; otherwise its
    /// local transform stays as it is. Fails when the entity is unknown or the move would make it
    /// its own ancestor.
    pub fn reparent(
        world: &World,
        child: EntityGuid,
        new_parent: Option<EntityGuid>,
        keep_world: bool,
    ) -> Result<Self, UndoError> {
        let child_entity = resolve(world, child)?;
        let new_parent_entity = match new_parent {
            Some(guid) => Some(resolve(world, guid)?),
            None => None,
        };
        if let Some(parent) = new_parent_entity
            && creates_cycle(world, child_entity, parent)
        {
            return Err(UndoError::Cycle);
        }
        let old_parent = world
            .get_component::<Parent>(child_entity)
            .and_then(|parent| GuidResolver::guid_of(world, parent.0));
        let old_local = world
            .get_component::<Transform>(child_entity)
            .copied()
            .unwrap_or_default();
        let new_local = if keep_world {
            let parent_world = new_parent_entity.map_or(Mat4::IDENTITY, |p| world_matrix(world, p));
            let local_matrix = parent_world.inverse() * world_matrix(world, child_entity);
            let (scale, rotation, translation) = local_matrix.to_scale_rotation_translation();
            Transform {
                translation,
                rotation,
                scale,
            }
        } else {
            old_local
        };
        Ok(Self::Reparent {
            child,
            old_parent,
            new_parent,
            keep_world,
            old_local,
            new_local,
        })
    }

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
            Self::Reparent {
                child,
                new_parent,
                new_local,
                ..
            } => {
                let child_entity = resolve(world, *child)?;
                let new_parent_entity = match new_parent {
                    Some(guid) => Some(resolve(world, *guid)?),
                    None => None,
                };
                if let Some(parent) = new_parent_entity
                    && creates_cycle(world, child_entity, parent)
                {
                    return Err(UndoError::Cycle);
                }
                detach(world, child_entity);
                if let Some(parent) = new_parent_entity {
                    set_parent(world, child_entity, parent);
                }
                set_transform(world, child_entity, *new_local);
                Ok(())
            }
            Self::Rename { guid, after, .. } => {
                let entity = resolve(world, *guid)?;
                set_name(world, entity, after.as_deref());
                Ok(())
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
            Self::Reparent {
                child,
                old_parent,
                new_parent,
                keep_world,
                old_local,
                new_local,
            } => Self::Reparent {
                child: *child,
                old_parent: *new_parent,
                new_parent: *old_parent,
                keep_world: *keep_world,
                old_local: *new_local,
                new_local: *old_local,
            },
            Self::Rename { guid, before, after } => Self::Rename {
                guid: *guid,
                before: after.clone(),
                after: before.clone(),
            },
        }
    }
}

fn resolve(world: &World, guid: EntityGuid) -> Result<Entity, UndoError> {
    GuidResolver::resolve(world, guid).ok_or(UndoError::EntityNotFound(guid))
}

/// Whether making `new_parent` the parent of `child` would make `child` its own ancestor.
fn creates_cycle(world: &World, child: Entity, new_parent: Entity) -> bool {
    let mut current = Some(new_parent);
    while let Some(entity) = current {
        if entity == child {
            return true;
        }
        current = world.get_component::<Parent>(entity).map(|parent| parent.0);
    }
    false
}

/// World matrix of `entity`: the product of the local matrices from the root down to it.
fn world_matrix(world: &World, entity: Entity) -> Mat4 {
    let local = world
        .get_component::<Transform>(entity)
        .map_or(Mat4::IDENTITY, |t| t.matrix());
    match world.get_component::<Parent>(entity) {
        Some(parent) => world_matrix(world, parent.0) * local,
        None => local,
    }
}

/// Removes the entity's parent link and drops it from the parent's children list.
fn detach(world: &mut World, child: Entity) {
    let Some(old_parent) = world.get_component::<Parent>(child).map(|parent| parent.0) else {
        return;
    };
    if let Some(children) = world.get_component_mut::<Children>(old_parent) {
        children.list.retain(|entity| *entity != child);
    }
    world.remove_component::<Parent>(child);
}

fn set_transform(world: &mut World, entity: Entity, value: Transform) {
    match world.get_component_mut::<Transform>(entity) {
        Some(transform) => *transform = value,
        None => world.add_component(entity, value),
    }
}

fn set_name(world: &mut World, entity: Entity, name: Option<&str>) {
    match name {
        Some(name) => match world.get_component_mut::<Name>(entity) {
            Some(component) => component.0 = name.to_string(),
            None => world.add_component(entity, Name(name.to_string())),
        },
        None => {
            world.remove_component::<Name>(entity);
        }
    }
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

/// One step of the history: a command with its label and the time it was recorded.
#[derive(Clone, Debug)]
pub struct UndoEntry {
    /// Name shown in the Edit menu and in the Undo History panel, such as "Move" or "Delete Entity".
    pub label: String,
    /// The command as it was applied (undo applies its inverse).
    pub command: UndoCommand,
    /// When the step was recorded (or last merged into).
    pub time: Instant,
}

/// A transaction in progress: nested `begin` calls only raise `depth`.
#[derive(Clone, Debug)]
struct Transaction {
    depth: usize,
    label: String,
    commands: Vec<UndoCommand>,
}

/// Undo and redo history with named steps, transactions and merging of rapid edits.
#[derive(Clone)]
pub struct UndoStack {
    /// Applied steps, oldest first.
    done: Vec<UndoEntry>,
    /// Undone steps, the next one to redo is last.
    undone: Vec<UndoEntry>,
    /// Maximum number of steps kept.
    limit: usize,
    /// Edits of the same value closer than this many seconds merge into one step.
    merge_window_secs: f32,
    /// Source of the time for merging.
    clock: Arc<dyn Clock>,
    /// When the last step was recorded.
    last_push: Option<Instant>,
    /// The transaction in progress, if any.
    transaction: Option<Transaction>,
    /// Number of applied steps when the document was last saved; `None` when that state can no
    /// longer be reached (its step was cut off by the limit or discarded by a new edit).
    saved_at: Option<usize>,
    /// Error of the last undo or redo that could not be applied.
    pub last_error: Option<UndoError>,
}

impl Default for UndoStack {
    fn default() -> Self {
        Self::new(DEFAULT_LIMIT)
    }
}

impl UndoStack {
    /// Creates a stack that keeps up to `limit` steps and reads the system clock.
    pub fn new(limit: usize) -> Self {
        Self::with_clock(limit, Arc::new(SystemClock))
    }

    /// Creates a stack that keeps up to `limit` steps and takes its time from `clock`.
    pub fn with_clock(limit: usize, clock: Arc<dyn Clock>) -> Self {
        Self {
            done: Vec::new(),
            undone: Vec::new(),
            limit,
            merge_window_secs: DEFAULT_MERGE_WINDOW_SECS,
            clock,
            last_push: None,
            transaction: None,
            saved_at: Some(0),
            last_error: None,
        }
    }

    /// Remembers the current state as the saved one.
    pub fn mark_saved(&mut self) {
        self.saved_at = Some(self.done.len());
    }

    /// Whether the current state differs from the last saved one.
    pub fn is_dirty(&self) -> bool {
        self.saved_at != Some(self.done.len())
    }

    /// Forgets the saved state when a new edit discards the redo branch that contains it.
    fn forget_saved_redo(&mut self) {
        if self.saved_at.is_some_and(|saved| saved > self.done.len()) {
            self.saved_at = None;
        }
    }

    /// Records a command that was already applied to the world. Inside a transaction the command
    /// joins the transaction; outside it becomes a step of its own (merged with the previous step
    /// when both edit the same value within the merge window).
    pub fn push(&mut self, cmd: UndoCommand) {
        if let Some(transaction) = self.transaction.as_mut() {
            add_to_transaction(transaction, cmd);
            return;
        }
        let label = default_label(&cmd);
        self.record(label, cmd, true);
    }

    /// Starts a transaction named `label`. Calls nest: only the outermost `commit` records the
    /// step. The label of the outermost call is used.
    pub fn begin(&mut self, label: impl Into<String>) {
        match self.transaction.as_mut() {
            Some(transaction) => transaction.depth += 1,
            None => {
                self.transaction = Some(Transaction {
                    depth: 1,
                    label: label.into(),
                    commands: Vec::new(),
                });
            }
        }
    }

    /// Ends the innermost transaction. The outermost one becomes one step with its label (no step
    /// when it recorded nothing). Returns `false` when no transaction is open.
    pub fn commit(&mut self) -> bool {
        let Some(transaction) = self.transaction.as_mut() else {
            return false;
        };
        transaction.depth = transaction.depth.saturating_sub(1);
        if transaction.depth > 0 {
            return true;
        }
        if let Some(finished) = self.transaction.take()
            && !finished.commands.is_empty()
        {
            // Stored as a batch even with one command, so that a later edit never merges into it.
            let command = UndoCommand::Batch(finished.commands);
            self.record(finished.label, command, false);
        }
        true
    }

    /// Ends the transaction and reverts the commands it recorded, newest first, leaving no step.
    /// Returns `false` when no transaction is open.
    pub fn cancel(&mut self, world: &mut World, registry: &TypeRegistry) -> bool {
        let Some(transaction) = self.transaction.take() else {
            return false;
        };
        for command in transaction.commands.iter().rev() {
            if let Err(error) = command.invert().apply(world, registry) {
                self.last_error = Some(error);
            }
        }
        true
    }

    /// Whether a transaction is open.
    pub fn in_transaction(&self) -> bool {
        self.transaction.is_some()
    }

    /// Number of steps that can be undone.
    pub fn len(&self) -> usize {
        self.done.len()
    }

    /// Whether there are no steps to undo.
    pub fn is_empty(&self) -> bool {
        self.done.is_empty()
    }

    /// The steps that can be undone, oldest first.
    pub fn entries(&self) -> &[UndoEntry] {
        &self.done
    }

    /// The steps that can be redone, the next one to redo last.
    pub fn redo_entries(&self) -> &[UndoEntry] {
        &self.undone
    }

    /// Maximum number of steps kept.
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Stores a step, merging it into the previous one when allowed. Clears the redo history.
    fn record(&mut self, label: String, command: UndoCommand, may_merge: bool) {
        let now = self.clock.now();
        self.forget_saved_redo();
        self.undone.clear();
        if may_merge && self.merge_into_last(&command, now) {
            self.last_push = Some(now);
            return;
        }
        self.done.push(UndoEntry {
            label,
            command,
            time: now,
        });
        self.last_push = Some(now);
        self.trim();
    }

    /// Replaces the value of the last step with `cmd`'s value when both edit the same component of
    /// the same entity within the merge window. The merged step keeps the original `before`.
    fn merge_into_last(&mut self, cmd: &UndoCommand, now: Instant) -> bool {
        let UndoCommand::SetComponent {
            guid,
            type_name,
            after,
            ..
        } = cmd
        else {
            return false;
        };
        if !self.within_merge_window(now) {
            return false;
        }
        let Some(entry) = self.done.last_mut() else {
            return false;
        };
        let UndoCommand::SetComponent {
            guid: last_guid,
            type_name: last_type,
            after: last_after,
            ..
        } = &mut entry.command
        else {
            return false;
        };
        if *last_guid != *guid || *last_type != *type_name {
            return false;
        }
        *last_after = after.clone();
        entry.time = now;
        true
    }

    /// Whether the last recorded step happened less than the merge window before `now`.
    fn within_merge_window(&self, now: Instant) -> bool {
        self.last_push.is_some_and(|last| {
            now.saturating_duration_since(last).as_secs_f32() < self.merge_window_secs
        })
    }

    /// Drops the oldest steps beyond the limit.
    fn trim(&mut self) {
        while self.done.len() > self.limit {
            self.done.remove(0);
            // Positions shift down by one; the saved state is lost when it was the first one.
            self.saved_at = self.saved_at.and_then(|saved| saved.checked_sub(1));
        }
    }

    /// Whether there is something to undo.
    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    /// Whether there is something to redo.
    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    /// Undoes the last step; returns whether anything was undone. A step that cannot be applied
    /// stays in the history and its error is stored in [`UndoStack::last_error`].
    pub fn undo(&mut self, world: &mut World, registry: &TypeRegistry) -> bool {
        let Some(inverse) = self.done.last().map(|entry| entry.command.invert()) else {
            return false;
        };
        if let Err(error) = inverse.apply(world, registry) {
            self.last_error = Some(error);
            return false;
        }
        if let Some(entry) = self.done.pop() {
            self.undone.push(entry);
        }
        true
    }

    /// Redoes the last undone step; returns whether anything was redone. A step that cannot be
    /// applied stays in the redo history and its error is stored in [`UndoStack::last_error`].
    pub fn redo(&mut self, world: &mut World, registry: &TypeRegistry) -> bool {
        let Some(entry) = self.undone.last() else {
            return false;
        };
        if let Err(error) = entry.command.apply(world, registry) {
            self.last_error = Some(error);
            return false;
        }
        if let Some(entry) = self.undone.pop() {
            self.done.push(entry);
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

/// Adds a command to an open transaction. A command that edits the same component of the same
/// entity as the previous one replaces its value, so a drag keeps one command per property.
fn add_to_transaction(transaction: &mut Transaction, cmd: UndoCommand) {
    let coalesced = match (transaction.commands.last_mut(), &cmd) {
        (
            Some(UndoCommand::SetComponent {
                guid: last_guid,
                type_name: last_type,
                after: last_after,
                ..
            }),
            UndoCommand::SetComponent {
                guid,
                type_name,
                after,
                ..
            },
        ) if *last_guid == *guid && *last_type == *type_name => {
            *last_after = after.clone();
            true
        }
        _ => false,
    };
    if !coalesced {
        transaction.commands.push(cmd);
    }
}

/// Label of a step pushed without an explicit transaction.
fn default_label(cmd: &UndoCommand) -> String {
    match cmd {
        UndoCommand::SetComponent { type_name, .. } => format!("Edit {type_name}"),
        UndoCommand::Spawn { .. } => "Create Entity".into(),
        UndoCommand::Despawn { .. } => "Delete Entity".into(),
        UndoCommand::Batch(_) => "Edit".into(),
        UndoCommand::Reparent { .. } => "Reparent Entity".into(),
        UndoCommand::Rename { .. } => "Rename Entity".into(),
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
