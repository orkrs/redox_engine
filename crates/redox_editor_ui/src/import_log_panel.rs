//! Import log tab (EDT-B4 step 4).

use redox_editor_assets::{ImportLogFilter, ImportLogModel, ImportLogStatus};

/// Panel showing import queue history.
pub struct ImportLogPanel {
    /// Filter selection mirrored for egui widgets.
    filter: ImportLogFilter,
    /// Path the browser asked to focus.
    pub focus_path: Option<String>,
}

impl ImportLogPanel {
    /// Creates the panel shell; render with [`Self::show`].
    pub fn new() -> Self {
        Self {
            filter: ImportLogFilter::default(),
            focus_path: None,
        }
    }

    /// Highlights one asset path.
    pub fn focus(&mut self, path: impl Into<String>) {
        self.focus_path = Some(path.into());
    }

    /// Renders the log; returns a path to retry when the user clicks Retry.
    pub fn show(&mut self, ui: &mut egui::Ui, model: &mut ImportLogModel) -> Option<String> {
        ui.horizontal(|ui| {
            ui.label("Filter");
            ui.selectable_value(&mut self.filter, ImportLogFilter::All, "all");
            ui.selectable_value(&mut self.filter, ImportLogFilter::Active, "active");
            ui.selectable_value(&mut self.filter, ImportLogFilter::Problems, "problems");
        });
        model.set_filter(self.filter);
        let mut retry = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for row in model.visible() {
                let highlight = self.focus_path.as_deref() == Some(row.path.as_str());
                let label = format!("{} — {}", row.path, status_label(&row.status));
                ui.horizontal(|ui| {
                    ui.selectable_label(highlight, label);
                    if matches!(
                        row.status,
                        ImportLogStatus::Failed(_) | ImportLogStatus::Cancelled
                    ) && ui.button("Retry").clicked()
                    {
                        retry = Some(row.path.clone());
                    }
                });
            }
        });
        retry
    }
}

impl Default for ImportLogPanel {
    fn default() -> Self {
        Self::new()
    }
}

fn status_label(status: &ImportLogStatus) -> &'static str {
    match status {
        ImportLogStatus::Started => "started",
        ImportLogStatus::Progress(_) => "progress",
        ImportLogStatus::Finished => "finished",
        ImportLogStatus::Failed(_) => "failed",
        ImportLogStatus::Cancelled => "cancelled",
    }
}