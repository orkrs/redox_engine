//! Inspector panel: draws an [`InspectorModel`] for the selection and turns edits into undo steps.

mod widgets;

use std::collections::HashSet;

use egui::{Color32, Id, Ui};
use redox_ecs::{Entity, World};
use redox_editor_core::inspector_model::{InspectorModel, Row, RowValue, addable_types};
use redox_editor_core::{EditorLog, UndoCommand, UndoStack};
use redox_gameplay::AttributeSet;
use redox_reflect::meta::FieldError;
use redox_reflect::{ReflectValue, TypeRegistry};

/// Panel state kept between frames.
#[derive(Clone, Default)]
struct PanelState {
    filter: String,
    show_advanced: bool,
    add_filter: String,
}

/// What the user asked for on one row this frame.
enum RowAction {
    None,
    /// Set the field to this value.
    Set(ReflectValue),
    /// Point an asset reference at this path.
    AssignPath(String),
    /// Restore the field's default.
    Reset,
}

/// Panel that edits the components of the selected entities.
pub struct InspectorPanel;

impl InspectorPanel {
    /// Draws the selection (`entities`, primary first) and records edits on `undo`.
    ///
    /// Edits are applied after the frame is drawn and recorded as one undo step each: a single
    /// command when it touches one entity, a batch when it touches several.
    pub fn show(
        ui: &mut Ui,
        registry: &TypeRegistry,
        entities: &[Entity],
        world: &mut World,
        undo: &mut UndoStack,
        log: &EditorLog,
    ) {
        if entities.is_empty() {
            ui.label("No selection");
            return;
        }
        let state_id = Id::new("inspector_panel_state");
        let mut state: PanelState = ui.data(|d| d.get_temp(state_id)).unwrap_or_default();
        let commands = draw(ui, registry, entities, world, &mut state, log);
        ui.data_mut(|d| d.insert_temp(state_id, state));
        commit(registry, world, undo, commands);
    }
}

/// The selection in inspector order: the primary entity first, then the rest by id.
pub fn selection_entities(primary: Option<Entity>, set: &HashSet<Entity>) -> Vec<Entity> {
    let mut rest: Vec<Entity> = set
        .iter()
        .copied()
        .filter(|entity| Some(*entity) != primary)
        .collect();
    rest.sort();
    primary.into_iter().chain(rest).collect()
}

/// Draws the panel and returns the commands the user asked for this frame.
fn draw(
    ui: &mut Ui,
    registry: &TypeRegistry,
    entities: &[Entity],
    world: &mut World,
    state: &mut PanelState,
    log: &EditorLog,
) -> Vec<UndoCommand> {
    let world_ref: &World = world;
    let mut pending = Vec::new();
    header(ui, entities, world_ref);

    ui.horizontal(|ui| {
        ui.label("🔍");
        ui.text_edit_singleline(&mut state.filter);
        ui.checkbox(&mut state.show_advanced, "Advanced");
    });
    let model = InspectorModel::build(
        registry,
        world_ref,
        entities,
        &state.filter,
        state.show_advanced,
    );
    if !model.partial_components.is_empty() {
        ui.colored_label(
            Color32::GRAY,
            format!(
                "Only on some selected objects: {}",
                model.partial_components.join(", ")
            ),
        );
    }

    for section in &model.sections {
        let title = match section.category {
            Some(category) => format!("{category} · {}", section.component),
            None => section.component.clone(),
        };
        egui::CollapsingHeader::new(title)
            .id_salt(section.component.as_str())
            .default_open(true)
            .show(ui, |ui| {
                if ui.small_button("Reset component").clicked() {
                    let result = model.reset_component(registry, world_ref, &section.component);
                    push_result(&mut pending, log, result);
                }
                for row in &section.rows {
                    let first = first_value(&model, registry, world_ref, row);
                    let action = draw_row(ui, row, first);
                    let result = match action {
                        RowAction::None => continue,
                        RowAction::Set(value) => {
                            model.edit(registry, world_ref, &row.component, row.field, value)
                        }
                        RowAction::AssignPath(path) => model.assign_asset_path(
                            registry,
                            world_ref,
                            &row.component,
                            row.field,
                            &path,
                        ),
                        RowAction::Reset => {
                            model.reset_field(registry, world_ref, &row.component, row.field)
                        }
                    };
                    push_result(&mut pending, log, result);
                }
            });
    }

    ui.separator();
    let mut add_request: Option<&'static str> = None;
    ui.menu_button("➕ Add Component", |ui| {
        ui.text_edit_singleline(&mut state.add_filter);
        for addable in addable_types(registry, world_ref, entities, &state.add_filter, false) {
            let label = match addable.category {
                Some(category) => format!("{} ({category})", addable.type_name),
                None => addable.type_name.to_string(),
            };
            if ui.button(label).clicked() {
                add_request = Some(addable.type_name);
                ui.close();
            }
        }
    });
    if let Some(component) = add_request {
        match model.add_component(registry, world_ref, component) {
            // undo: n/a
            Ok(outcome) => {
                for note in &outcome.notes {
                    log.info(note);
                }
                pending.extend(outcome.commands);
            }
            Err(error) => log.warn(format!("Add Component: {error}")),
        }
    }
    pending
}

/// Entity line and the GAS summary of the primary entity.
fn header(ui: &mut Ui, entities: &[Entity], world: &World) {
    let primary = entities[0];
    if entities.len() == 1 {
        ui.label(format!("📦 Entity: #{}", primary.id()));
    } else {
        ui.label(format!(
            "📦 {} selected, primary #{}",
            entities.len(),
            primary.id()
        ));
    }
    if let Some(attrs) = world.get_component::<AttributeSet>(primary) {
        ui.label(format!(
            "GAS  HP {:.0}/{:.0}  STA {:.0}/{:.0}",
            attrs.health.current, attrs.health.max, attrs.stamina.current, attrs.stamina.max
        ));
    }
}

/// The value of a mixed row on the primary entity, for the "= first" button.
fn first_value(
    model: &InspectorModel,
    registry: &TypeRegistry,
    world: &World,
    row: &Row,
) -> Option<ReflectValue> {
    if !row.mixed {
        return None;
    }
    let primary = *model.entities.first()?;
    registry
        .read_component(world, primary, &row.component)?
        .field(row.field)
        .cloned()
}

/// One row: label, value widget, modified marker and reset button.
fn draw_row(ui: &mut Ui, row: &Row, first: Option<ReflectValue>) -> RowAction {
    let mut action = RowAction::None;
    ui.horizontal(|ui| {
        let title = if row.modified_from_default {
            format!("● {}", row.label)
        } else {
            row.label.clone()
        };
        match row.tooltip {
            Some(tooltip) => ui.label(title).on_hover_text(tooltip),
            None => ui.label(title),
        };
        match &row.value {
            RowValue::Single(value) if value.as_handle_path().is_some() => {
                let path = ui
                    .add_enabled_ui(!row.read_only, |ui| widgets::asset_widget(ui, row, value))
                    .inner;
                if let Some(path) = path {
                    action = RowAction::AssignPath(path);
                }
            }
            RowValue::Single(value) => {
                let edited = ui
                    .add_enabled_ui(!row.read_only, |ui| widgets::value_widget(ui, row, value))
                    .inner;
                if let Some(value) = edited {
                    action = RowAction::Set(value);
                }
            }
            RowValue::Mixed => {
                ui.label("—");
                if let Some(value) = first.filter(|_| !row.read_only)
                    && ui.small_button("= first").clicked()
                {
                    action = RowAction::Set(value);
                }
            }
        }
        if !row.read_only
            && ui
                .small_button("↺")
                .on_hover_text("Reset to default")
                .clicked()
        {
            action = RowAction::Reset;
        }
    });
    action
}

/// Adds the result of an edit to the frame's commands, or logs why it failed.
fn push_result(
    pending: &mut Vec<UndoCommand>,
    log: &EditorLog,
    result: Result<Vec<UndoCommand>, FieldError>,
) {
    match result {
        Ok(commands) => pending.extend(commands),
        Err(error) => log.warn(format!("Inspector: {error}")),
    }
}

/// Applies the commands and records them as one undo step.
fn commit(
    registry: &TypeRegistry,
    world: &mut World,
    undo: &mut UndoStack,
    commands: Vec<UndoCommand>,
) {
    if commands.is_empty() {
        return;
    }
    for command in &commands {
        let _ = command.apply(world, registry);
    }
    let entry = match <[UndoCommand; 1]>::try_from(commands) {
        Ok([single]) => single,
        Err(commands) => UndoCommand::Batch(commands),
    };
    undo.push(entry);
}
