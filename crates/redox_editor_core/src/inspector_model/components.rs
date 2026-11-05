//! Adding and removing components on the selection, and the list of types the add menu offers.
//!
//! Adding a component also adds the components it `requires` that are missing; each one is
//! reported in [`AddOutcome::notes`] so the caller can write it to the console. Removing a
//! component that another component requires is allowed, but produces a warning.

use redox_ecs::{Entity, World};
use redox_reflect::TypeRegistry;
use redox_reflect::meta::FieldError;

use crate::undo::UndoCommand;

/// Commands that add a component, with the notes to show for automatically added requirements.
#[derive(Clone, Debug, Default)]
pub struct AddOutcome {
    /// One `SetComponent` per entity and component added. Requirements come before the components
    /// that need them.
    pub commands: Vec<UndoCommand>,
    /// Console lines, one per requirement added automatically.
    pub notes: Vec<String>,
}

/// Commands that remove a component, with warnings about components left without a requirement.
#[derive(Clone, Debug, Default)]
pub struct RemoveOutcome {
    /// One `SetComponent` with `after: None` per entity that had the component.
    pub commands: Vec<UndoCommand>,
    /// Warnings naming components that require the removed one.
    pub warnings: Vec<String>,
}

/// A component type the add menu can offer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddableType {
    /// Registered name of the type.
    pub type_name: &'static str,
    /// Category of the type, if any.
    pub category: Option<&'static str>,
}

impl super::InspectorModel {
    /// Commands that add `component` to every selected entity that lacks it, together with the
    /// components it requires. Entities that already have a component are left alone.
    pub fn add_component(
        &self,
        registry: &TypeRegistry,
        world: &World,
        component: &str,
    ) -> Result<AddOutcome, FieldError> {
        let mut outcome = AddOutcome::default();
        for &entity in &self.entities {
            let mut planned = Vec::new();
            plan_component(
                registry,
                world,
                entity,
                component,
                None,
                &mut planned,
                &mut outcome,
            )?;
        }
        Ok(outcome)
    }

    /// Commands that remove `component` from every selected entity that has it, and warnings for
    /// components on those entities that require it.
    pub fn remove_component(
        &self,
        registry: &TypeRegistry,
        world: &World,
        component: &str,
    ) -> Result<RemoveOutcome, FieldError> {
        if registry.schema(component).is_none() {
            return Err(FieldError::UnknownType(component.to_string()));
        }
        let mut outcome = RemoveOutcome::default();
        for &entity in &self.entities {
            let Some(before) = registry.read_component(world, entity, component) else {
                continue;
            };
            outcome.commands.push(UndoCommand::SetComponent {
                entity,
                type_name: component.to_string(),
                before: Some(before),
                after: None,
            });
            for other in registry.type_names() {
                if other == component || registry.read_component(world, entity, other).is_none() {
                    continue;
                }
                let requires = registry.schema(other).is_some_and(|schema| {
                    schema
                        .meta
                        .requires
                        .iter()
                        .any(|required| *required == component)
                });
                if requires {
                    outcome.warnings.push(format!(
                        "{other} on #{} requires {component}; removing it may break {other}",
                        entity.id()
                    ));
                }
            }
        }
        outcome.warnings.sort();
        outcome.warnings.dedup();
        Ok(outcome)
    }
}

/// Types the add menu offers for the entities: the ones not present on all of them, matching
/// `filter` (case-insensitive substring of the name or category). Editor-only types are included
/// only when `include_editor_only` is set. Sorted by category, then name.
pub fn addable_types(
    registry: &TypeRegistry,
    world: &World,
    entities: &[Entity],
    filter: &str,
    include_editor_only: bool,
) -> Vec<AddableType> {
    let needle = filter.trim().to_lowercase();
    let mut types: Vec<AddableType> = registry
        .type_names()
        .filter_map(|name| {
            let schema = registry.schema(name)?;
            if schema.meta.editor_only && !include_editor_only {
                return None;
            }
            let present_everywhere = !entities.is_empty()
                && entities
                    .iter()
                    .all(|&entity| registry.read_component(world, entity, name).is_some());
            if present_everywhere {
                return None;
            }
            let category = schema.meta.category;
            let matches = needle.is_empty()
                || name.to_lowercase().contains(&needle)
                || category.is_some_and(|c| c.to_lowercase().contains(&needle));
            matches.then_some(AddableType {
                type_name: name,
                category,
            })
        })
        .collect();
    types.sort_by(|a, b| {
        (a.category.is_none(), a.category.unwrap_or(""))
            .cmp(&(b.category.is_none(), b.category.unwrap_or("")))
            .then_with(|| a.type_name.cmp(b.type_name))
    });
    types
}

/// Adds `name` to `entity` unless it is already there or planned, then plans its requirements
/// first. `required_by` names the component that needs `name`, for the note.
fn plan_component(
    registry: &TypeRegistry,
    world: &World,
    entity: Entity,
    name: &str,
    required_by: Option<&str>,
    planned: &mut Vec<String>,
    outcome: &mut AddOutcome,
) -> Result<(), FieldError> {
    if planned.iter().any(|p| p == name) || registry.read_component(world, entity, name).is_some()
    {
        return Ok(());
    }
    let schema = registry
        .schema(name)
        .ok_or_else(|| FieldError::UnknownType(name.to_string()))?;
    let value = registry
        .default_value(name)
        .ok_or_else(|| FieldError::UnknownType(name.to_string()))?;
    planned.push(name.to_string());
    for &required in schema.meta.requires {
        plan_component(
            registry,
            world,
            entity,
            required,
            Some(name),
            planned,
            outcome,
        )?;
    }
    if let Some(parent) = required_by {
        outcome.notes.push(format!(
            "Added {name} to #{} (required by {parent})",
            entity.id()
        ));
    }
    outcome.commands.push(UndoCommand::SetComponent {
        entity,
        type_name: name.to_string(),
        before: None,
        after: Some(value),
    });
    Ok(())
}
