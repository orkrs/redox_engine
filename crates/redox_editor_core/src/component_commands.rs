//! Add, remove and copy component values as one undo transaction each.

use redox_ecs::{Entity, World};
use redox_reflect::meta::FieldError;
use redox_reflect::TypeRegistry;

use crate::component_catalog::{
    DependencyPromptAnswer, DependencyPromptOutcome, DependencyWarning, dependency_warning_for_add,
    resolve_dependency_prompt,
};
use crate::inspector_model::{
    AddOutcome, InspectorModel, RemoveOutcome, copy_component, paste_component_values,
};
use crate::undo::UndoCommand;
use crate::undo::UndoStack;

/// Result of planning or applying a component add.
#[derive(Clone, Debug)]
pub enum ComponentAddResult {
    /// Commands ready to record; `warnings` are dependency notes for the log.
    Ready {
        /// Undo commands for the add (including auto-added `requires`).
        commands: Vec<UndoCommand>,
        /// Console lines from auto-added requirements.
        notes: Vec<String>,
        /// Lines to log when the user continued past a soft-dependency prompt.
        warnings: Vec<String>,
    },
    /// The UI must show a Continue/Cancel prompt first.
    NeedsPrompt(DependencyWarning),
    /// The user cancelled the add.
    Cancelled,
}

/// Plans adding `type_name` to `entities`. When a soft-dependency prompt is needed and `answer` is
/// `None`, returns [`ComponentAddResult::NeedsPrompt`]. Pass `Some(Continue)` or `Some(Cancel)`
/// after the user chooses.
pub fn component_add(
    registry: &TypeRegistry,
    world: &World,
    entities: &[Entity],
    type_name: &str,
    answer: Option<DependencyPromptAnswer>,
) -> Result<ComponentAddResult, FieldError> {
    if let Some(warning) = dependency_warning_for_add(registry, world, entities, type_name) {
        match answer {
            None => return Ok(ComponentAddResult::NeedsPrompt(warning)),
            Some(choice) => {
                if resolve_dependency_prompt(choice) == DependencyPromptOutcome::Abort {
                    return Ok(ComponentAddResult::Cancelled);
                }
                let model = inspector_for(entities);
                let outcome = model.add_component(registry, world, type_name)?;
                return Ok(ComponentAddResult::Ready {
                    commands: outcome.commands,
                    notes: outcome.notes,
                    warnings: vec![warning.message],
                });
            }
        }
    }
    let model = inspector_for(entities);
    let outcome = model.add_component(registry, world, type_name)?;
    Ok(ComponentAddResult::Ready {
        commands: outcome.commands,
        notes: outcome.notes,
        warnings: Vec::new(),
    })
}

/// Commands that remove `type_name` from every selected entity that has it.
pub fn component_remove(
    registry: &TypeRegistry,
    world: &World,
    entities: &[Entity],
    type_name: &str,
) -> Result<RemoveOutcome, FieldError> {
    inspector_for(entities).remove_component(registry, world, type_name)
}

/// Copies field values of `component` from `source` onto every entity in `targets` that has
/// `component`. Only fields with the same name and type are copied.
pub fn component_copy_values(
    registry: &TypeRegistry,
    world: &World,
    source: Entity,
    targets: &[Entity],
    component: &str,
) -> Result<Vec<UndoCommand>, FieldError> {
    let Some(value) = copy_component(registry, world, source, component) else {
        return Err(FieldError::MissingComponent(component.to_string()));
    };
    paste_component_values(registry, world, targets, component, &value)
}

/// Applies `commands` to `world` and records them as one undo step named `label`.
pub fn record_component_transaction(
    undo: &mut UndoStack,
    registry: &TypeRegistry,
    world: &mut World,
    label: &str,
    commands: Vec<UndoCommand>,
) -> Result<(), FieldError> {
    if commands.is_empty() {
        return Ok(());
    }
    undo.begin(label);
    for command in &commands {
        command
            .apply(world, registry)
            .map_err(|error| FieldError::Reflect(error.to_string()))?;
        undo.push(command.clone());
    }
    undo.commit();
    Ok(())
}

/// Convenience: add + one transaction (skips when `answer` is pending or cancelled).
pub fn component_add_and_record(
    undo: &mut UndoStack,
    registry: &TypeRegistry,
    world: &mut World,
    entities: &[Entity],
    type_name: &str,
    answer: Option<DependencyPromptAnswer>,
    log_warn: &mut dyn FnMut(&str),
    log_info: &mut dyn FnMut(&str),
) -> Result<bool, FieldError> {
    match component_add(registry, world, entities, type_name, answer)? {
        ComponentAddResult::NeedsPrompt(_) | ComponentAddResult::Cancelled => return Ok(false),
        ComponentAddResult::Ready {
            commands,
            notes,
            warnings,
        } => {
            for line in &warnings {
                log_warn(line);
            }
            for line in &notes {
                log_info(line);
            }
            let label = format!("Add {type_name}");
            record_component_transaction(undo, registry, world, &label, commands)?;
            Ok(true)
        }
    }
}

fn inspector_for(entities: &[Entity]) -> InspectorModel {
    InspectorModel {
        entities: entities.to_vec(),
        ..InspectorModel::default()
    }
}

/// Outcome of a bulk add for tests and callers that need command counts.
pub struct BulkAddOutcome {
    /// Commands produced (one batch per entity and requirement).
    pub add: AddOutcome,
}

/// Adds `type_name` to every entity in the model that lacks it (existing components are skipped).
pub fn bulk_add_component(
    model: &InspectorModel,
    registry: &TypeRegistry,
    world: &World,
    type_name: &str,
) -> Result<BulkAddOutcome, FieldError> {
    let add = model.add_component(registry, world, type_name)?;
    Ok(BulkAddOutcome { add })
}