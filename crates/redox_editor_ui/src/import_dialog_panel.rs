//! Import dialog window (EDT-B4 step 2).

use std::path::PathBuf;

use redox_editor_assets::{
    ImportDialogModel, ImportDialogPhase, ImportPreview, ImportPresetId,
};

/// Shell around [`ImportDialogModel`].
pub struct ImportDialogPanel {
    /// Pure dialog model.
    pub model: ImportDialogModel,
    /// Paths chosen by the file picker.
    pending_paths: Vec<PathBuf>,
}

impl ImportDialogPanel {
    /// Empty dialog.
    pub fn new() -> Self {
        Self {
            model: ImportDialogModel::default(),
            pending_paths: Vec::new(),
        }
    }

    /// Queues paths from a picker.
    pub fn set_paths(&mut self, paths: Vec<PathBuf>, assets_root: &std::path::Path) {
        self.pending_paths = paths;
        self.model = ImportDialogModel::begin_paths(self.pending_paths.clone(), assets_root);
    }

    /// Whether the dialog should paint.
    pub fn is_open(&self) -> bool {
        self.model.is_open()
    }

    /// Renders the dialog; returns confirmed jobs on OK.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        assets_root: &std::path::Path,
    ) -> Option<Vec<redox_editor_assets::ImportDialogJob>> {
        if !self.is_open() {
            return None;
        }
        let mut jobs = None;
        let mut open = true;
        egui::Window::new("Import assets")
            .collapsible(false)
            .resizable(true)
            .open(&mut open)
            .show(ctx, |ui| {
                if let ImportDialogPhase::Preview(rows) = &self.model.phase {
                    for (index, row) in rows.iter().enumerate() {
                        ui.separator();
                        ui.label(&row.rel_path);
                        ui.label(&row.probe_summary);
                        match &row.preview {
                            ImportPreview::TextureHeader { width, height, format } => {
                                ui.label(format!("preview {width}x{height} {format}"));
                            }
                            ImportPreview::MeshSilhouetteDeferred => {
                                ui.label("mesh silhouette: pending EDT-B3");
                            }
                            ImportPreview::None => {}
                        }
                        let preset = row.settings.preset;
                        egui::ComboBox::from_id_salt(format!("import_preset_{index}"))
                            .selected_text(preset.name())
                            .show_ui(ui, |ui| {
                                for id in ImportPresetId::all() {
                                    if ui
                                        .selectable_label(preset == *id, id.name())
                                        .clicked()
                                    {
                                        let _ = self.model.set_preset(index, *id);
                                    }
                                }
                            });
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Import").clicked() {
                            jobs = Some(self.model.confirm());
                        }
                        if ui.button("Cancel").clicked() {
                            let _ = self.model.cancel();
                            open = false;
                        }
                    });
                } else if !open {
                    let _ = self.model.cancel();
                } else {
                    ui.label("No importable files selected.");
                }
            });
        if !open {
            let _ = self.model.cancel();
        }
        jobs
    }
}

impl Default for ImportDialogPanel {
    fn default() -> Self {
        Self::new()
    }
}