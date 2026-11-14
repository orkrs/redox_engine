//! Material editor side panel: errors, parameters, WGSL and preview options.

use redox_editor_assets::{PreviewLighting, PreviewMesh};
use redox_matgraph::NodeId;

use crate::material_editor::MaterialEditorSession;

/// UI for one material session (Graph tab, Material mode).
pub struct MaterialPanel;

impl MaterialPanel {
    /// Draws compile errors, params, preview toggles and read-only WGSL.
    pub fn show(
        ui: &mut egui::Ui,
        session: &mut MaterialEditorSession,
        preview_tex: Option<egui::TextureId>,
    ) -> MaterialPanelAction {
        let mut action = MaterialPanelAction::None;
        ui.horizontal(|ui| {
            ui.heading(session.title());
            if ui.button("Save").clicked() {
                action = MaterialPanelAction::Save;
            }
            if ui.button("Refresh compile").clicked() {
                session.refresh_compile();
            }
            if session.view.undo.can_undo() && ui.button("Undo graph").clicked() {
                action = MaterialPanelAction::Undo;
            }
        });
        ui.separator();
        ui.collapsing("Preview", |ui| {
            egui::ComboBox::from_id_salt("mat_preview_mesh")
                .selected_text(preview_mesh_label(session.preview.mesh))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut session.preview.mesh, PreviewMesh::Sphere, "Sphere");
                    ui.selectable_value(&mut session.preview.mesh, PreviewMesh::Cube, "Cube");
                    ui.selectable_value(&mut session.preview.mesh, PreviewMesh::Plane, "Plane");
                });
            egui::ComboBox::from_id_salt("mat_preview_light")
                .selected_text(preview_light_label(session.preview.lighting))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut session.preview.lighting,
                        PreviewLighting::Simple,
                        "Simple light",
                    );
                    ui.selectable_value(
                        &mut session.preview.lighting,
                        PreviewLighting::ViewportIbl,
                        "Viewport IBL (B3)",
                    );
                });
            if let Some(tex) = preview_tex {
                ui.image(egui::load::SizedTexture::new(tex, egui::vec2(128.0, 128.0)));
            }
        });
        ui.collapsing("Errors", |ui| {
            if session.compile.errors.is_empty() {
                ui.label("No compile errors");
            } else {
                let errors: Vec<(Option<u32>, String)> = session
                    .compile
                    .errors
                    .iter()
                    .map(|err| (err.node, err.message.clone()))
                    .collect();
                for (i, (node, message)) in errors.iter().enumerate() {
                    let label = if let Some(n) = node {
                        format!("#{n}: {message}")
                    } else {
                        message.clone()
                    };
                    if ui.selectable_label(false, label).clicked() {
                        session.focus_error(i);
                        action = MaterialPanelAction::FocusNode;
                    }
                }
            }
        });
        ui.collapsing("Parameters", |ui| {
            let params = session.param_nodes();
            if params.is_empty() {
                ui.label("(no Param nodes)");
            }
            for (id, name, value) in params {
                ui.label(&name);
                let mut value = value;
                let mut changed = false;
                ui.horizontal(|ui| {
                    changed |= ui
                        .add(egui::DragValue::new(&mut value[0]).speed(0.01))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(&mut value[1]).speed(0.01))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(&mut value[2]).speed(0.01))
                        .changed();
                });
                if changed {
                    session.set_param(id, value);
                }
            }
        });
        ui.collapsing("Generated WGSL", |ui| {
            egui::ScrollArea::vertical()
                .max_height(180.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut session.wgsl_display)
                            .desired_width(f32::INFINITY)
                            .interactive(false),
                    );
                });
        });
        action
    }
}

/// Side effects requested by the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MaterialPanelAction {
    /// Nothing.
    #[default]
    None,
    /// Save the material document.
    Save,
    /// Undo the last graph edit.
    Undo,
    /// Focus node changed — refresh canvas highlight.
    FocusNode,
}

fn preview_mesh_label(mesh: PreviewMesh) -> &'static str {
    match mesh {
        PreviewMesh::Sphere => "Sphere",
        PreviewMesh::Cube => "Cube",
        PreviewMesh::Plane => "Plane",
    }
}

fn preview_light_label(light: PreviewLighting) -> &'static str {
    match light {
        PreviewLighting::Simple => "Simple light",
        PreviewLighting::ViewportIbl => "Viewport IBL",
    }
}
