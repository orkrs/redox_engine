//! Undo-safe outliner edits (reparent, hide, lock, isolate).

use std::collections::HashSet;

use redox_ecs::{Entity, World};
use redox_editor_core::markers::Locked;
use redox_editor_core::selection::Visibility;
use redox_editor_core::undo::{UndoCommand, UndoError, UndoStack};
use redox_reflect::{Reflect, TypeRegistry};
use redox_scene::EntityGuid;

use crate::guid_index::GuidIndex;

/// Plans and applies a reparent in one named undo step.
pub fn reparent(
    stack: &mut UndoStack,
    world: &mut World,
    registry: &TypeRegistry,
    child: EntityGuid,
    new_parent: Option<EntityGuid>,
    keep_world: bool,
) -> Result<(), UndoError> {
    let cmd = UndoCommand::reparent(world, child, new_parent, keep_world)?;
    stack.begin("Reparent");
    cmd.apply(world, registry)?;
    stack.push(cmd);
    stack.commit();
    Ok(())
}

/// Builds an undo command that sets visibility on `guid`.
pub fn visibility_command(
    world: &World,
    guid: EntityGuid,
    visible: bool,
) -> Result<UndoCommand, UndoError> {
    let entity = redox_editor_core::guid::GuidResolver::resolve(world, guid)
        .ok_or(UndoError::EntityNotFound(guid))?;
    let before = world
        .get_component::<Visibility>(entity)
        .map(|v| v.to_value());
    let after = Visibility(visible).to_value();
    Ok(UndoCommand::SetComponent {
        guid,
        type_name: "Visibility".into(),
        before,
        after: Some(after),
    })
}

/// Applies hide or show on one entity inside a transaction the caller may batch.
pub fn apply_visibility(
    world: &mut World,
    registry: &TypeRegistry,
    stack: &mut UndoStack,
    guid: EntityGuid,
    visible: bool,
    label: &str,
) -> Result<(), UndoError> {
    let cmd = visibility_command(world, guid, visible)?;
    stack.begin(label);
    cmd.apply(world, registry)?;
    stack.push(cmd);
    stack.commit();
    Ok(())
}

/// Builds a lock toggle command (`lock = true` adds [`Locked`], `false` removes it).
pub fn lock_command(world: &World, guid: EntityGuid, lock: bool) -> Result<UndoCommand, UndoError> {
    let entity = redox_editor_core::guid::GuidResolver::resolve(world, guid)
        .ok_or(UndoError::EntityNotFound(guid))?;
    let before = world
        .get_component::<Locked>(entity)
        .map(|_| Locked.to_value());
    let after = lock.then(|| Locked.to_value());
    Ok(UndoCommand::SetComponent {
        guid,
        type_name: "Locked".into(),
        before,
        after,
    })
}

/// Applies lock state as one undo step.
pub fn apply_lock(
    world: &mut World,
    registry: &TypeRegistry,
    stack: &mut UndoStack,
    guid: EntityGuid,
    lock: bool,
) -> Result<(), UndoError> {
    let cmd = lock_command(world, guid, lock)?;
    stack.begin(if lock { "Lock" } else { "Unlock" });
    cmd.apply(world, registry)?;
    stack.push(cmd);
    stack.commit();
    Ok(())
}

/// Hides every entity that is not in `selected` as one undo step named "Isolate".
pub fn isolate(
    stack: &mut UndoStack,
    world: &mut World,
    registry: &TypeRegistry,
    index: &GuidIndex,
    selected: &HashSet<Entity>,
) -> Result<(), UndoError> {
    let mut commands = Vec::new();
    for entity in world.all_entities() {
        if selected.contains(&entity) {
            continue;
        }
        let Some(guid) = index.guid_of(entity) else {
            continue;
        };
        let hidden = world
            .get_component::<Visibility>(entity)
            .is_some_and(|v| !v.0);
        if hidden {
            continue;
        }
        commands.push(visibility_command(world, guid, false)?);
    }
    if commands.is_empty() {
        return Ok(());
    }
    stack.begin("Isolate");
    for cmd in &commands {
        cmd.apply(world, registry)?;
        stack.push(cmd.clone());
    }
    stack.commit();
    Ok(())
}

/// Whether the entity can be picked in the viewport (hidden entities are not pickable).
pub fn is_pickable(world: &World, entity: Entity) -> bool {
    !world
        .get_component::<Visibility>(entity)
        .is_some_and(|v| !v.0)
}

/// Whether the entity is marked hidden in the outliner sense.
pub fn is_hidden(world: &World, entity: Entity) -> bool {
    world
        .get_component::<Visibility>(entity)
        .is_some_and(|v| !v.0)
}

/// Whether the entity is locked.
pub fn is_locked(world: &World, entity: Entity) -> bool {
    world.get_component::<Locked>(entity).is_some()
}
