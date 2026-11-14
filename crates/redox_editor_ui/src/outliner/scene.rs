//! Scene hierarchy outliner with virtual list, DnD reparent and context menu.

use redox_ecs::Entity;
use redox_ecs::World;
use redox_editor_core::EditorContext;
use redox_editor_core::Selection;
use redox_editor_core::guid::GuidResolver;
use redox_editor_core::undo::UndoStack;
use redox_editor_scene::guid_index::GuidIndex;
use redox_editor_scene::outliner_model::{
    FlatRow, OutlinerFilter, OUTLINER_ROW_HEIGHT, SelectionModifiers, build, flatten,
    invert_selection, select_children, select_range, visible_row_range,
};
use redox_editor_scene::outliner_ops::{apply_lock, apply_visibility, isolate, reparent as reparent_undo};
use redox_reflect::TypeRegistry;
use redox_scene::EntityGuid;

use super::OutlinerAction;
use super::OutlinerPanel;

/// Drag payload type for reparent.
#[derive(Clone, Copy, Debug)]
pub struct OutlinerDragPayload(pub EntityGuid);

/// Draws the scene tree outliner.
pub fn show_scene(
    ui: &mut egui::Ui,
    panel: &mut OutlinerPanel,
    world: &World,
    registry: &TypeRegistry,
    selection: &mut Selection,
    undo: &mut UndoStack,
    core: &EditorContext,
    index: &GuidIndex,
) -> Vec<OutlinerAction> {
    let mut actions = Vec::new();
    let error_guids = &panel.validation_errors;
    let model = build(world, index, error_guids);
    if !model.warnings.is_empty() && panel.warned_count < model.warnings.len() {
        panel.warned_count = model.warnings.len();
        for warning in &model.warnings {
            core.log
                .warn(format!("Outliner: {} ({})", warning.message, warning.guid));
        }
    }
    let filter = OutlinerFilter {
        name: panel.filter.clone(),
        component: panel.component_filter.clone(),
        errors_only: panel.errors_only,
    };
    let flat = flatten(
        &model,
        world,
        registry,
        &filter,
        error_guids,
        &panel.expanded,
    );
    panel.flat_len = flat.len();

    let row_height = OUTLINER_ROW_HEIGHT;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let viewport_h = ui.available_height().max(row_height);
            let scroll_y = ui.clip_rect().top() - ui.min_rect().top();
            let visible = visible_row_range(flat.len(), scroll_y.max(0.0), viewport_h, row_height);
            panel.last_rows_drawn = visible.rows_drawn;
            ui.add_space(visible.range.start as f32 * row_height);
            for row in flat[visible.range.clone()].iter() {
                if draw_row(
                    ui,
                    panel,
                    row,
                    &model,
                    &flat,
                    world,
                    selection,
                    undo,
                    core,
                    index,
                    &mut actions,
                ) {
                    // reparent was applied
                }
            }
            let tail = flat.len().saturating_sub(visible.range.end);
            ui.add_space(tail as f32 * row_height);
        });
    actions
}

#[allow(clippy::too_many_arguments)]
fn draw_row(
    ui: &mut egui::Ui,
    panel: &mut OutlinerPanel,
    row: &FlatRow,
    model: &redox_editor_scene::OutlinerModel,
    flat: &[FlatRow],
    world: &World,
    selection: &mut Selection,
    undo: &mut UndoStack,
    core: &EditorContext,
    index: &GuidIndex,
    actions: &mut Vec<OutlinerAction>,
) -> bool {
    let Some(entity) = index.get(row.guid) else {
        return false;
    };
    let Some(node) = model.nodes.get(&row.guid) else {
        return false;
    };
    let selected = selection.set.contains(&entity);
    let indent = row.depth as f32 * 14.0;
    let mut reparented = false;
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), OUTLINER_ROW_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.add_space(indent);
            if row.has_children {
                let open = panel.expanded.contains(&row.guid);
                let twisty = if open { "▼" } else { "▶" };
                if ui.small_button(twisty).clicked() {
                    if open {
                        panel.expanded.remove(&row.guid);
                    } else {
                        panel.expanded.insert(row.guid);
                    }
                }
            } else {
                ui.add_space(18.0);
            }
            let mut label = node.name.clone();
            if node.is_folder {
                label = format!("📁 {label}");
            }
            if node.hidden {
                label.push_str(" (hidden)");
            }
            if node.locked {
                label.push_str(" 🔒");
            }
            if node.has_error {
                label.push_str(" ⚠");
            }
            let response = ui.selectable_label(selected, label);
            if response.clicked() {
                let modifiers = ui.input(|input| SelectionModifiers {
                    ctrl: input.modifiers.ctrl || input.modifiers.mac_cmd,
                    shift: input.modifiers.shift,
                });
                if modifiers.shift {
                    if let Some(anchor) = panel.range_anchor {
                        select_range(selection, flat, index, anchor, row.guid);
                    } else {
                        selection.set_primary(Some(entity));
                    }
                } else if modifiers.ctrl {
                    selection.toggle(entity);
                    panel.range_anchor = Some(row.guid);
                } else {
                    selection.set_primary(Some(entity));
                    panel.range_anchor = Some(row.guid);
                }
            }
            response.context_menu(|ui| {
                scene_context_menu(ui, row.guid, entity, core, index, selection, model, flat, actions);
            });
            response.dnd_set_drag_payload(OutlinerDragPayload(row.guid));
            if let Some(payload) = response.dnd_release_payload::<OutlinerDragPayload>() {
                if payload.0 != row.guid {
                    let mut world_guard = core.world.lock();
                    let mut undo_guard = core.undo.lock();
                    let registry_guard = core.registry.lock();
                    match reparent_undo(
                        &mut undo_guard,
                        &mut world_guard,
                        &registry_guard,
                        payload.0,
                        Some(row.guid),
                        true,
                    ) {
                        Ok(()) => reparented = true,
                        Err(error) => core.log.warn(format!("Reparent failed: {error}")),
                    }
                }
            }
            let _ = world;
            let _ = undo;
        },
    );
    reparented
}

#[allow(clippy::too_many_arguments)]
fn scene_context_menu(
    ui: &mut egui::Ui,
    guid: EntityGuid,
    entity: Entity,
    core: &EditorContext,
    index: &GuidIndex,
    selection: &Selection,
    model: &redox_editor_scene::OutlinerModel,
    flat: &[FlatRow],
    actions: &mut Vec<OutlinerAction>,
) {
    if ui.button("Rename…").clicked() {
        panel_begin_rename(core, guid);
        ui.close();
    }
    if ui.button("Duplicate").clicked() {
        core.selection.lock().set_primary(Some(entity));
        let _ = core.commands.run(core, "duplicate");
        ui.close();
    }
    if ui.button("Delete").clicked() {
        core.selection.lock().set_primary(Some(entity));
        let _ = core.commands.run(core, "delete");
        ui.close();
    }
    ui.separator();
    if ui.button("Reparent to root").clicked() {
        let mut world = core.world.lock();
        let mut undo = core.undo.lock();
        let registry = core.registry.lock();
        let _ = reparent_undo(&mut undo, &mut world, &registry, guid, None, true);
        ui.close();
    }
    if ui.button("Hide").clicked() {
        let mut world = core.world.lock();
        let mut undo = core.undo.lock();
        let registry = core.registry.lock();
        let _ = apply_visibility(&mut world, &registry, &mut undo, guid, false, "Hide");
        ui.close();
    }
    if ui.button("Show").clicked() {
        let mut world = core.world.lock();
        let mut undo = core.undo.lock();
        let registry = core.registry.lock();
        let _ = apply_visibility(&mut world, &registry, &mut undo, guid, true, "Show");
        ui.close();
    }
    if ui.button("Lock").clicked() {
        let mut world = core.world.lock();
        let mut undo = core.undo.lock();
        let registry = core.registry.lock();
        let _ = apply_lock(&mut world, &registry, &mut undo, guid, true);
        ui.close();
    }
    if ui.button("Unlock").clicked() {
        let mut world = core.world.lock();
        let mut undo = core.undo.lock();
        let registry = core.registry.lock();
        let _ = apply_lock(&mut world, &registry, &mut undo, guid, false);
        ui.close();
    }
    if ui.button("Focus").clicked() {
        actions.push(OutlinerAction::FocusScene(guid));
        ui.close();
    }
    if ui.button("Select children").clicked() {
        let mut sel = core.selection.lock();
        select_children(&mut sel, model, index, guid, true);
        ui.close();
    }
    if ui.button("Invert selection").clicked() {
        let mut sel = core.selection.lock();
        invert_selection(&mut sel, flat, index);
        ui.close();
    }
    if ui.button("Isolate selection").clicked() {
        let mut world = core.world.lock();
        let mut undo = core.undo.lock();
        let registry = core.registry.lock();
        let selected = core.selection.lock().set.clone();
        let _ = isolate(&mut undo, &mut world, &registry, index, &selected);
        ui.close();
    }
    let _ = selection;
}

fn panel_begin_rename(core: &EditorContext, guid: EntityGuid) {
    let entity = GuidResolver::resolve(&core.world.lock(), guid);
    let name = entity
        .and_then(|e| {
            core.world
                .lock()
                .get_component::<redox_scene::Name>(e)
                .map(|n| n.0.clone())
        })
        .unwrap_or_default();
    core.status
        .lock()
        .replace(format!("Rename entity {name} in the Inspector (F2)"));
}