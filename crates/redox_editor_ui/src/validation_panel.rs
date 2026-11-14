//! egui Issues tab (EDT-B7 step 3).

use redox_editor_core::guid::GuidResolver;
use redox_editor_core::Selection;
use redox_editor_scene::validate::{IssueUserAction, Severity};

use crate::validation_model::{SeverityFilter, ValidationPanelAction, ValidationPanelModel};

/// Draws the Issues tab; returns host actions (selection changes, open paths).
pub struct ValidationPanel;

impl ValidationPanel {
    /// Shows the panel and may queue Select / Open / Fixed outcomes.
    pub fn show(
        ui: &mut egui::Ui,
        model: &mut ValidationPanelModel,
        selection: &mut Selection,
        world: &mut redox_ecs::World,
        registry: &redox_reflect::TypeRegistry,
        undo: &mut redox_editor_core::UndoStack,
    ) -> (Vec<ValidationPanelAction>, bool) {
        let mut actions = Vec::new();
        let mut refresh = false;
        ui.horizontal(|ui| {
            ui.label("Severity:");
            egui::ComboBox::from_id_salt("validation_severity_filter")
                .selected_text(filter_label(model.filter))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut model.filter, SeverityFilter::All, "All");
                    ui.selectable_value(
                        &mut model.filter,
                        SeverityFilter::WarnPlus,
                        "Warn+",
                    );
                    ui.selectable_value(&mut model.filter, SeverityFilter::Errors, "Errors");
                });
            if ui.button("Re-validate").clicked() {
                refresh = true;
            }
        });
        ui.separator();
        if model.report.issues.is_empty() {
            ui.label("No issues.");
            return (actions, refresh);
        }
        let rows: Vec<_> = model
            .filtered_issues()
            .into_iter()
            .cloned()
            .collect();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for issue in &rows {
                let color = match issue.severity {
                    Severity::Error => egui::Color32::from_rgb(220, 100, 90),
                    Severity::Warning => egui::Color32::from_rgb(220, 180, 80),
                    Severity::Info => egui::Color32::from_rgb(140, 170, 200),
                };
                ui.horizontal(|ui| {
                    ui.colored_label(color, format!("{:?}", issue.severity));
                    ui.label(&issue.message);
                });
                ui.horizontal(|ui| {
                    if issue.select.is_some() || issue.entity.is_some() {
                        if ui.small_button("Select").clicked() {
                            if let Ok(act) = model.apply_action(
                                issue,
                                IssueUserAction::Select,
                                world,
                                registry,
                                undo,
                            ) {
                                if let ValidationPanelAction::Select(guid) = act {
                                    if let Some(entity) =
                                        GuidResolver::resolve(world, guid)
                                    {
                                        selection.set_primary(Some(entity));
                                    }
                                    actions.push(act);
                                }
                            }
                        }
                    }
                    if issue.open.is_some() || issue.asset_path.is_some() {
                        if ui.small_button("Open").clicked() {
                            if let Ok(act) = model.apply_action(
                                issue,
                                IssueUserAction::Open,
                                world,
                                registry,
                                undo,
                            ) {
                                actions.push(act);
                            }
                        }
                    }
                    if issue.fix.is_some() && ui.small_button("Fix").clicked() {
                        if let Ok(act) = model.apply_action(
                            issue,
                            IssueUserAction::Fix,
                            world,
                            registry,
                            undo,
                        ) {
                            actions.push(act);
                        }
                    }
                });
                ui.separator();
            }
        });
        (actions, refresh)
    }
}

fn filter_label(filter: SeverityFilter) -> &'static str {
    match filter {
        SeverityFilter::All => "All",
        SeverityFilter::WarnPlus => "Warn+",
        SeverityFilter::Errors => "Errors",
    }
}