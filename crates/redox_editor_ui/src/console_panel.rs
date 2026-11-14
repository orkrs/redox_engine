//! Console dock tab with filters, export and GUID click-through.

use redox_editor_core::EditorContext;
use redox_editor_debugger::{ConsoleModel, LogLevel, search_runtime_entities};
use redox_scene::components::EntityGuid;

use crate::context::EditorUiCtx;

/// Console panel state (filter lives on [`EditorUiCtx`]).
pub struct ConsolePanel;

impl ConsolePanel {
    /// Draws the console with filter controls.
    pub fn show(ui: &mut egui::Ui, ctx: &EditorUiCtx, core: &EditorContext) {
        let mut filter = ctx.console_filter.lock();
        let mut search = ctx.runtime_search.lock();
        ui.horizontal(|ui| {
            ui.label("Min:");
            egui::ComboBox::from_id_salt("console_level")
                .selected_text(format!("{:?}", filter.min_level))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut filter.min_level, LogLevel::Info, "Info");
                    ui.selectable_value(&mut filter.min_level, LogLevel::Warn, "Warn");
                    ui.selectable_value(&mut filter.min_level, LogLevel::Error, "Error");
                });
            ui.label("Category:");
            ui.add(
                egui::TextEdit::singleline(filter.category.get_or_insert_with(String::new))
                    .desired_width(80.0),
            );
            ui.label("Text:");
            ui.text_edit_singleline(&mut filter.text);
            if ui.button("Clear filter").clicked() {
                *filter = Default::default();
            }
            if ui.button("Export…").clicked() {
                let model = ConsoleModel::from_lines(&core.log.lines());
                let path = std::path::Path::new("console_export.log");
                if let Err(err) = model.export_filtered(&filter, path) {
                    core.log.error(format!("console export failed: {err}"));
                } else {
                    core.log
                        .info(format!("console exported to {}", path.display()));
                }
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Runtime search:");
            ui.text_edit_singleline(&mut search.text);
            if ui.button("Find").clicked() {
                let world = core.world.lock();
                let hits = search_runtime_entities(&world, &search);
                if let Some(hit) = hits.first() {
                    *ctx.runtime_highlight.lock() =
                        redox_editor_debugger::RuntimeInspectorHighlight {
                            entity: Some(hit.entity),
                        };
                    *ctx.debug_flags.lock() = {
                        let mut flags = *ctx.debug_flags.lock();
                        flags.highlight_runtime_entity = Some(hit.entity.id());
                        flags
                    };
                    core.selection.lock().set_primary(Some(hit.entity));
                }
            }
        });
        ui.separator();
        let model = ConsoleModel::from_lines(&core.log.lines());
        let entries: Vec<_> = model.filtered(&filter).cloned().collect();
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for (index, entry) in entries.iter().enumerate() {
                    let label = if let Some(cat) = &entry.category {
                        format!("[{:?}][{cat}] {}", entry.level, entry.message)
                    } else {
                        format!("[{:?}] {}", entry.level, entry.message)
                    };
                    let response = ui.selectable_label(false, label);
                    if (response.clicked() || response.double_clicked())
                        && let Some(guid) = entry
                            .entity_guid
                            .as_deref()
                            .or_else(|| model.filtered_guid_at(&filter, index))
                    {
                        Self::select_by_guid(core, guid);
                    }
                }
            });
    }

    fn select_by_guid(core: &EditorContext, guid_hex: &str) {
        let world = core.world.lock();
        let parsed = u128::from_str_radix(guid_hex, 16).ok();
        let Some(bits) = parsed else {
            return;
        };
        for entity in world.all_entities() {
            if world
                .get_component::<EntityGuid>(entity)
                .is_some_and(|g| g.0 == bits)
            {
                core.selection.lock().set_primary(Some(entity));
                break;
            }
        }
    }
}
