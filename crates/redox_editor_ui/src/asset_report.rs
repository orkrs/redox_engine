//! Diagnostics panel for the report of opening an assets root (EDT-A6 step 6): damaged sidecars,
//! scene references to missing assets, unreadable scenes, and GUIDs that were regenerated.
//! The panel only reports actions; the app applies them through the asset service.

use redox_asset::AssetGuid;
use redox_editor_assets::{AssetReport, MissingRef, RefFix};

/// An action chosen in the diagnostics panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportAction {
    /// Select the asset at this path (relative to the root) in the browser.
    SelectPath(String),
    /// Open the scene at this path (relative to the root).
    OpenScene(String),
    /// Remove the damaged sidecar and register the file again.
    RepairSidecar(String),
    /// Fix one missing reference in its scene.
    Fix {
        /// The reference to fix.
        target: MissingRef,
        /// The fix to apply.
        fix: RefFix,
    },
}

/// Severity of a diagnostic line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// Something is broken and needs a fix.
    Error,
    /// Something is missing, the project still works.
    Warning,
    /// Information about what was done automatically.
    Info,
}

impl Severity {
    fn label(self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Warning => "WARN",
            Self::Info => "INFO",
        }
    }

    fn color(self) -> egui::Color32 {
        match self {
            Self::Error => egui::Color32::from_rgb(220, 120, 100),
            Self::Warning => egui::Color32::from_rgb(220, 180, 90),
            Self::Info => egui::Color32::from_rgb(140, 170, 200),
        }
    }
}

/// Whether a report has problems worth showing as soon as a project opens.
pub fn has_problems(report: &AssetReport) -> bool {
    !report.damaged.is_empty()
        || !report.missing_refs.is_empty()
        || !report.unreadable_scenes.is_empty()
}

/// The window with the diagnostics of the last open or rescan.
#[derive(Debug, Default)]
pub struct AssetReportPanel {
    /// Whether the window is shown.
    pub open: bool,
}

impl AssetReportPanel {
    /// Shows the window and returns the actions chosen in it. `selected` is the asset selected in
    /// the browser; it is the target of "Reassign".
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        report: &AssetReport,
        selected: Option<AssetGuid>,
    ) -> Vec<ReportAction> {
        let mut actions = Vec::new();
        if !self.open {
            return actions;
        }
        let mut open = self.open;
        egui::Window::new("Asset diagnostics")
            .open(&mut open)
            .default_width(520.0)
            .show(ctx, |ui| {
                ui.label(format!(
                    "{} damaged · {} missing references · {} unreadable scenes · {} sidecars created · {} GUIDs regenerated",
                    report.damaged.len(),
                    report.missing_refs.len(),
                    report.unreadable_scenes.len(),
                    report.created.len(),
                    report.regenerated.len()
                ));
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(380.0)
                    .show(ui, |ui| {
                        damaged_rows(ui, report, &mut actions);
                        missing_rows(ui, report, selected, &mut actions);
                        unreadable_rows(ui, report);
                        regenerated_rows(ui, report, &mut actions);
                    });
            });
        self.open = open;
        actions
    }
}

fn damaged_rows(ui: &mut egui::Ui, report: &AssetReport, actions: &mut Vec<ReportAction>) {
    for path in &report.damaged {
        let rel = path.to_string_lossy().replace('\\', "/");
        ui.horizontal(|ui| {
            ui.colored_label(Severity::Error.color(), Severity::Error.label());
            ui.label(format!("{rel}: damaged .meta, the asset is not registered"));
            if ui.small_button("Recreate sidecar").clicked() {
                actions.push(ReportAction::RepairSidecar(rel.clone()));
            }
        });
    }
}

fn missing_rows(
    ui: &mut egui::Ui,
    report: &AssetReport,
    selected: Option<AssetGuid>,
    actions: &mut Vec<ReportAction>,
) {
    for missing in &report.missing_refs {
        let target = missing
            .path
            .as_deref()
            .or(missing.guid.as_deref())
            .unwrap_or("-");
        let entity = missing.entity.as_deref().unwrap_or("-");
        ui.horizontal(|ui| {
            ui.colored_label(Severity::Warning.color(), Severity::Warning.label());
            ui.label(format!(
                "{} · entity {} · {}: asset {} not found",
                missing.scene, entity, missing.component, target
            ));
            if ui.small_button("Select scene").clicked() {
                actions.push(ReportAction::SelectPath(missing.scene.clone()));
            }
            if ui.small_button("Open scene").clicked() {
                actions.push(ReportAction::OpenScene(missing.scene.clone()));
            }
            let can_reassign = selected.is_some();
            if ui
                .add_enabled(can_reassign, egui::Button::new("Reassign to selected"))
                .clicked()
                && let Some(guid) = selected
            {
                actions.push(ReportAction::Fix {
                    target: missing.clone(),
                    fix: RefFix::Reassign(guid),
                });
            }
            if ui.small_button("Remove reference").clicked() {
                actions.push(ReportAction::Fix {
                    target: missing.clone(),
                    fix: RefFix::Clear,
                });
            }
        });
    }
}

fn unreadable_rows(ui: &mut egui::Ui, report: &AssetReport) {
    for text in &report.unreadable_scenes {
        ui.horizontal(|ui| {
            ui.colored_label(Severity::Error.color(), Severity::Error.label());
            ui.label(text.as_str());
        });
    }
}

fn regenerated_rows(ui: &mut egui::Ui, report: &AssetReport, actions: &mut Vec<ReportAction>) {
    for path in &report.regenerated {
        let rel = path.to_string_lossy().replace('\\', "/");
        ui.horizontal(|ui| {
            ui.colored_label(Severity::Info.color(), Severity::Info.label());
            ui.label(format!("{rel}: copy received a new GUID; references stay on the original"));
            if ui.small_button("Select").clicked() {
                actions.push(ReportAction::SelectPath(rel.clone()));
            }
        });
    }
}
