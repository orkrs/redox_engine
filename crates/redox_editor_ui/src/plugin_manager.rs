//! Project → Plugins: list, enable, safe mode banner.

use egui;
use redox_editor_core::project::ProjectContext;
use redox_editor_core::settings::PluginEditorSettings;
use redox_editor_core::{CommandRegistry, EditorContext};
use redox_editor_plugins::{PluginHost, load_native_plugin};

/// Window for managing project plugins.
#[derive(Debug, Default)]
pub struct PluginManagerWindow {
    open: bool,
    draft_plugins: Vec<String>,
    draft_safe_mode: bool,
    native_warning_shown: bool,
}

impl PluginManagerWindow {
    /// Opens the manager for the current project.
    pub fn open_for_project(&mut self, project: &ProjectContext, safe_mode: PluginEditorSettings) {
        self.draft_plugins = project.manifest().plugins.clone();
        self.draft_safe_mode = safe_mode.isolate_plugin_panics;
        self.open = true;
        self.native_warning_shown = false;
    }

    /// Whether the window is visible.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Draws the plugin manager and safe-mode banner. Returns updated project plugins and settings
    /// when the user saves.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        host: &mut PluginHost,
        project: Option<&mut ProjectContext>,
        editor: &EditorContext,
        commands: &mut CommandRegistry,
        safe_mode: &mut PluginEditorSettings,
    ) -> bool {
        let mut saved = false;
        if host.safe_mode_active {
            egui::Window::new("plugin_safe_mode_banner")
                .title_bar(false)
                .anchor(egui::Align2::CENTER_TOP, [0.0, 0.0])
                .frame(egui::Frame::NONE.fill(egui::Color32::from_rgb(120, 40, 40)))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let message = host
                            .safe_mode_message
                            .as_deref()
                            .unwrap_or("Safe mode: a plugin was disabled after a panic.");
                        ui.colored_label(egui::Color32::WHITE, message);
                        if ui.button("Dismiss").clicked() {
                            host.dismiss_safe_mode_banner();
                        }
                    });
                });
        }

        if !self.open {
            return false;
        }

        let mut close = false;
        let mut apply = false;
        egui::Window::new("Project Plugins")
            .collapsible(false)
            .resizable(true)
            .open(&mut self.open)
            .show(ctx, |ui| {
                ui.label("Built-in plugins (in-process). Native .dll loading is off by default.");
                if ui.button("Check native loader stub").clicked() {
                    match load_native_plugin(std::path::Path::new("plugin.dll")) {
                        Ok(_) => editor
                            .log
                            .info("[plugins] native stub returned Ok".to_string()),
                        Err(err) => {
                            editor.log.warn(format!("[plugins] native: {err}"));
                            self.native_warning_shown = true;
                        }
                    }
                }
                if self.native_warning_shown {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        "Native plugins are unsupported / may crash — use built-ins only.",
                    );
                }
                ui.separator();
                ui.checkbox(
                    &mut self.draft_safe_mode,
                    "Safe mode (disable plugin on panic)",
                );
                ui.separator();
                let catalog = host.catalog_names();
                for name in catalog {
                    let enabled = self.draft_plugins.iter().any(|p| p == &name);
                    let mut check = enabled;
                    if ui.checkbox(&mut check, name.clone()).changed() {
                        if check {
                            if !self.draft_plugins.iter().any(|p| p == &name) {
                                self.draft_plugins.push(name);
                            }
                        } else {
                            self.draft_plugins.retain(|p| p != &name);
                        }
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        apply = true;
                        close = true;
                    }
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            });
        if close {
            self.open = false;
        }
        if apply {
            if let Some(project) = project {
                project.manifest_mut().plugins = self.draft_plugins.clone();
                if let Err(err) = project.save_manifest() {
                    editor
                        .log
                        .error(format!("cannot save project plugins: {err}"));
                } else {
                    saved = true;
                }
            }
            *safe_mode = PluginEditorSettings {
                isolate_plugin_panics: self.draft_safe_mode,
            };
            host.apply_project_plugins(&self.draft_plugins, commands, &editor.log);
        }
        saved
    }
}
