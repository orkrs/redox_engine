//! Fix, Select and Open actions on validation issues.

use redox_ecs::hierarchy::{Children, Parent};
use redox_ecs::{Entity, World};
use redox_editor_core::guid::GuidResolver;
use redox_editor_core::undo::{UndoCommand, UndoError, UndoStack, capture_entity};
use redox_math::Transform;
use redox_reflect::TypeRegistry;
use redox_render::Camera;
use redox_scene::{EntityGuid, Name};

use super::issue::{ValidationFix, ValidationIssue};

/// Outcome of applying a user action to an issue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationActionResult {
    /// Selection should focus this entity.
    Select(EntityGuid),
    /// A document path should open in the asset browser or editor.
    Open(String),
    /// The fix was applied through `undo`.
    Fixed,
    /// Nothing to do for this action.
    Noop,
}

/// Applies [`ValidationFix`] in one undo transaction when possible.
pub fn apply_fix(
    fix: &ValidationFix,
    world: &mut World,
    registry: &TypeRegistry,
    undo: &mut UndoStack,
) -> Result<(), UndoError> {
    match fix {
        ValidationFix::AddDefaultCamera => {
            undo.begin("Add default camera");
            let entity = world.spawn(); // undo: tx
            let guid = EntityGuid::new_random();
            world.add_component(entity, guid); // undo: tx
            world.add_component(entity, Transform::default()); // undo: tx
            world.add_component(entity, Camera::default()); // undo: tx
            world.add_component(entity, Name("Camera".into())); // undo: tx
            let snapshot = capture_entity(world, entity, registry);
            undo.push(UndoCommand::Spawn { guid, snapshot });
            undo.commit();
            Ok(())
        }
        ValidationFix::Unparent { entity } => {
            let child =
                GuidResolver::resolve(world, *entity).ok_or(UndoError::EntityNotFound(*entity))?;
            let old_parent = world
                .get_component::<Parent>(child)
                .and_then(|parent| GuidResolver::guid_of(world, parent.0));
            let old_local = world
                .get_component::<Transform>(child)
                .copied()
                .unwrap_or_default();
            undo.begin("Break hierarchy cycle");
            detach_parent(world, child);
            undo.push(UndoCommand::Reparent {
                child: *entity,
                old_parent,
                new_parent: None,
                keep_world: true,
                old_local,
                new_local: old_local,
            });
            undo.commit();
            Ok(())
        }
    }
}

/// Runs Select, Open or Fix for one issue.
pub fn apply_issue_action(
    issue: &ValidationIssue,
    action: IssueUserAction,
    world: &mut World,
    registry: &TypeRegistry,
    undo: &mut UndoStack,
) -> Result<ValidationActionResult, UndoError> {
    match action {
        IssueUserAction::Select => {
            if let Some(guid) = issue.select.or(issue.entity) {
                Ok(ValidationActionResult::Select(guid))
            } else {
                Ok(ValidationActionResult::Noop)
            }
        }
        IssueUserAction::Open => {
            if let Some(path) = &issue.open {
                Ok(ValidationActionResult::Open(path.clone()))
            } else if let Some(path) = &issue.asset_path {
                Ok(ValidationActionResult::Open(path.clone()))
            } else {
                Ok(ValidationActionResult::Noop)
            }
        }
        IssueUserAction::Fix => {
            if let Some(fix) = &issue.fix {
                apply_fix(fix, world, registry, undo)?;
                Ok(ValidationActionResult::Fixed)
            } else {
                Ok(ValidationActionResult::Noop)
            }
        }
    }
}

fn detach_parent(world: &mut World, child: Entity) {
    let Some(Parent(old)) = world.get_component::<Parent>(child).copied() else {
        return;
    };
    world.remove_component::<Parent>(child); // undo: tx
    if let Some(mut kids) = world.get_component::<Children>(old).cloned() {
        kids.list.retain(|entity| *entity != child);
        world.add_component(old, kids); // undo: tx
    }
}

/// Button the user pressed on an issue row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueUserAction {
    /// Focus the entity in the outliner / viewport.
    Select,
    /// Open the related asset or scene document.
    Open,
    /// Apply the suggested fix.
    Fix,
}
