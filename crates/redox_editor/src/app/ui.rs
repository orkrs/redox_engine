//! egui layout: menu bar, status bar, dock and command palette.

use super::EditorApp;
use crate::commands::{focus_selection, run_command};
use redox_editor_core::EditorContext;
use redox_editor_core::autosave::AutosavePolicy;
use redox_editor_core::file_commands::FileRequest;
use redox_editor_scene::WorldSession;
use redox_editor_ui::{RecoveryAnswer, format_hhmm};
use redox_editor_ui::asset_report::ReportAction;
use redox_math::Transform;

/// Text of an Edit menu entry: the verb, the step label when there is one, and the shortcut.
fn step_text(verb: &str, label: Option<&str>, shortcut: &str) -> String {
    match label {
        Some(label) => format!("{verb} {label}  {shortcut}"),
        None => format!("{verb}  {shortcut}"),
    }
}

/// Applies one undo (or redo) step; a failure is written to the console.
fn history_step(core: &EditorContext, redo: bool) {
    let error = {
        let mut world = core.world.lock();
        let registry = core.registry.lock();
        let mut undo = core.undo.lock();
        if redo {
            undo.redo(&mut world, &registry);
        } else {
            undo.undo(&mut world, &registry);
        }
        undo.last_error.take()
    };
    if let Some(error) = error {
        core.log.error(format!("[ERROR] {error}"));
    }
}

impl EditorApp {
    pub(super) fn build_ui(&mut self) -> egui::FullOutput {
        let raw_input = self.egui_winit.take_egui_input(&self.window);
        let ui_ctx_ref = self.ui_ctx.clone();
        let mut recovery_answer: Option<RecoveryAnswer> = None;
        let mut preferences_answer: Option<AutosavePolicy> = None;
        let mut report_actions: Vec<ReportAction> = Vec::new();
        let output = self.egui_ctx.run_ui(raw_input, |ui| {
            let ctx = ui.ctx().clone();
            let ctx = &ctx;
            // Ctrl+P palette
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::P)) {
                ui_ctx_ref.palette.lock().toggle();
            }
            // Ctrl+S saves the level; Ctrl+Shift+S saves the layout.
            let (save_level_hotkey, save_layout_hotkey) = ctx.input(|i| {
                let s_pressed = i.modifiers.command && i.key_pressed(egui::Key::S);
                (s_pressed && !i.modifiers.shift, s_pressed && i.modifiers.shift)
            });
            if save_level_hotkey {
                self.files.push_request(FileRequest::Save);
            }
            if save_layout_hotkey {
                let _ = self
                    .dock
                    .save_to_file(std::path::Path::new("editor_layout.ron"));
            }

            egui::Panel::top("top_bar").show(ui, |ui| {
                ui.horizontal(|ui| {
                    let mut save_layout = false;
                    ui.menu_button("File", |ui| {
                        self.files.show_items(ui);
                        if ui.button("New World").clicked()
                            && let Ok(s) =
                                WorldSession::create_new("worlds/untitled", "empty", 4, 4)
                        {
                            *ui_ctx_ref.world_session.lock() = Some(s);
                            ui_ctx_ref.log("[INFO] Created worlds/untitled");
                        }
                        if ui.button("Open worlds/arena").clicked()
                            && let Ok(s) = WorldSession::open("worlds/arena")
                        {
                            *ui_ctx_ref.world_session.lock() = Some(s);
                            ui_ctx_ref.log("[INFO] Opened worlds/arena");
                        }
                        ui.separator();
                        if ui.button("Save Layout   Ctrl+Shift+S").clicked() {
                            save_layout = true;
                        }
                        if ui.button("Asset Diagnostics").clicked() {
                            self.asset_report.open = true;
                        }
                    });
                    if save_layout {
                        let _ = self
                            .dock
                            .save_to_file(std::path::Path::new("editor_layout.ron"));
                    }
                    ui.menu_button("Edit", |ui| {
                        let (undo_label, redo_label) = {
                            let undo = self.core.undo.lock();
                            (
                                undo.entries().last().map(|e| e.label.clone()),
                                undo.redo_entries().last().map(|e| e.label.clone()),
                            )
                        };
                        let undo_text = step_text("Undo", undo_label.as_deref(), "Ctrl+Z");
                        if ui
                            .add_enabled(undo_label.is_some(), egui::Button::new(undo_text))
                            .clicked()
                        {
                            history_step(&self.core, false);
                        }
                        let redo_text = step_text("Redo", redo_label.as_deref(), "Ctrl+Y");
                        if ui
                            .add_enabled(redo_label.is_some(), egui::Button::new(redo_text))
                            .clicked()
                        {
                            history_step(&self.core, true);
                        }
                        if ui.button("Command Palette Ctrl+P").clicked() {
                            ui_ctx_ref.palette.lock().toggle();
                        }
                        if ui.button("Preferences").clicked() {
                            self.preferences.open_with(self.settings.autosave);
                        }
                    });
                    ui.menu_button("Entity", |ui| {
                        if ui.button("Create Empty Ctrl+Shift+N").clicked() {
                            run_command(&ui_ctx_ref, &mut self.dock, "create_entity");
                        }
                        if ui.button("Duplicate Ctrl+D").clicked() {
                            run_command(&ui_ctx_ref, &mut self.dock, "duplicate");
                        }
                        if ui.button("Delete").clicked() {
                            run_command(&ui_ctx_ref, &mut self.dock, "delete");
                        }
                    });
                    ui.menu_button("World", |ui| {
                        // Agent E: list worlds/ via suggest_world_roots → open_world:
                        let roots = WorldSession::suggest_world_roots();
                        if roots.is_empty() {
                            ui.label("No worlds/ directories found");
                        } else {
                            for root in roots {
                                let label = format!("Open {}", root.display());
                                if ui.button(label).clicked() {
                                    *ui_ctx_ref.core.status.lock() =
                                        format!("open_world:{}", root.display());
                                    ui.close();
                                }
                            }
                        }
                        ui.separator();
                        if ui.button("Tick Streaming").clicked()
                            && let Some(session) = ui_ctx_ref.world_session.lock().as_mut()
                        {
                            let eye = self.camera.lock().eye;
                            session.tick(&mut self.core.world.lock(), eye);
                            ui_ctx_ref.log("[INFO] World partition tick");
                        }
                    });
                    ui.menu_button("Git", |ui| {
                        // Agent E: surface git2 availability + refresh
                        let git = ui_ctx_ref.git.lock();
                        ui.label(git.summary());
                        ui.label(if git.is_available() {
                            "editor-git: git2 compiled in"
                        } else {
                            "editor-git off — OFPA dirty tracking only"
                        });
                        drop(git);
                        if ui.button("Refresh status").clicked() {
                            *ui_ctx_ref.core.status.lock() = "git_refresh".into();
                            ui.close();
                        }
                    });
                    ui.menu_button("View", |ui| {
                        if ui.button("Focus Selection F").clicked() {
                            focus_selection(&ui_ctx_ref);
                        }
                        if ui.button("Quad View (F5)").clicked() {
                            let mut l = ui_ctx_ref.layout.lock();
                            *l = l.toggle();
                        }
                        if ui.button("Reset Layout").clicked() {
                            run_command(&ui_ctx_ref, &mut self.dock, "reset_layout");
                        }
                    });
                    ui.menu_button("Build", |ui| {
                        if ui.button("Bake Meshlets").clicked() {
                            run_command(&ui_ctx_ref, &mut self.dock, "bake_meshlets");
                        }
                        if ui.button("Write Bake Report").clicked() {
                            run_command(&ui_ctx_ref, &mut self.dock, "bake_report");
                        }
                        if ui.button("Spawn StateTree Agent").clicked() {
                            run_command(&ui_ctx_ref, &mut self.dock, "spawn_agent");
                        }
                    });
                    ui.menu_button("Help", |ui| {
                        if ui.button("About").clicked() {
                            ui_ctx_ref.log("[INFO] RedOx Studio — MSAA+SMAA editor shell");
                        }
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let pie = ui_ctx_ref.pie.lock();
                        let running = pie.running;
                        let paused = pie.paused;
                        drop(pie);

                        if ui.button("⏹").clicked() {
                            run_command(&ui_ctx_ref, &mut self.dock, "pie_stop");
                        }
                        if ui.button("⏸").clicked() {
                            let mut pie = ui_ctx_ref.pie.lock();
                            if pie.running {
                                if pie.paused {
                                    pie.resume();
                                } else {
                                    pie.pause();
                                }
                            }
                        }
                        let play_label = if running && !paused {
                            "▶ Playing"
                        } else if paused {
                            "▶ Resume"
                        } else {
                            "▶ Play"
                        };
                        if ui.button(play_label).clicked() {
                            run_command(&ui_ctx_ref, &mut self.dock, "pie_play");
                        }
                        ui.separator();
                        let sel = self.core.selection.lock().primary.map(|e| e.id());
                        ui.label(format!("Sel:{:?}", sel));
                        ui.label(format!("Gizmo:{:?}", self.gizmo.lock().mode));
                    });
                });
            });

            egui::Panel::bottom("status_bar").show(ui, |ui| {
                ui.horizontal(|ui| {
                    let git = if ui_ctx_ref.world_session.lock().is_some() {
                        format!("🟢 {}", ui_ctx_ref.git.lock().summary())
                    } else {
                        "⚪ No world".into()
                    };
                    ui.label(git);
                    ui.separator();
                    if let Some(service) = &self.assets {
                        let counts = service.import_counts();
                        ui.label(format!(
                            "Importing {} / Failed {}",
                            counts.importing, counts.failed
                        ));
                        ui.separator();
                    }
                    let fps = *ui_ctx_ref.fps.lock();
                    let ms = *ui_ctx_ref.frame_ms.lock();
                    let low1 = ui_ctx_ref.profiler.lock().one_percent_low_fps();
                    ui.label(format!("FPS: {fps:.0} ({ms:.1}ms) · 1% {low1:.0}"));
                    ui.separator();
                    ui.label(format!("{:?}", *ui_ctx_ref.shading.lock()));
                    ui.separator();
                    ui.label(ui_ctx_ref.vram_label.lock().clone());
                    ui.separator();
                    let cell = self
                        .core
                        .selection
                        .lock()
                        .primary
                        .and_then(|e| {
                            self.core
                                .world
                                .lock()
                                .get_component::<Transform>(e)
                                .map(|t| {
                                    format!(
                                        "Cell: ({:.0},{:.0})",
                                        (t.translation.x / 16.0).floor(),
                                        (t.translation.z / 16.0).floor()
                                    )
                                })
                        })
                        .unwrap_or_else(|| "Cell: (0,0)".into());
                    ui.label(cell);
                    ui.separator();
                    if let Some(secs) = self.autosave.as_ref().and_then(|s| s.last_saved_secs()) {
                        ui.label(format!("Autosaved {}", format_hhmm(secs)));
                        ui.separator();
                    }
                    if let Some(note) = self.recovery_note.as_deref() {
                        ui.label(note);
                        ui.separator();
                    }
                    ui.label(format!("Undo: {}", self.core.undo.lock().len()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let snap = *ui_ctx_ref.surface_snap.lock();
                        ui.label(if snap {
                            "Surface Snap: ON"
                        } else {
                            "Surface Snap: OFF"
                        });
                    });
                });
            });

            self.files.handle_requests(&self.core);
            if let Some(title) = self.files.take_title_change(&self.core) {
                self.window.set_title(&title);
            }
            self.files.show_windows(ctx, &self.core);

            self.dock.show_with_ui(
                ui,
                &ui_ctx_ref,
                self.viewport_tex_id,
                Some(self.viewport_rect.clone()),
            );

            if self.asset_report.open
                && let Some(service) = self.assets.as_ref()
            {
                let mut report = service.report().clone();
                report.missing_refs = service.missing_refs();
                let selected = ui_ctx_ref.assets.lock().model.selected().map(|e| e.guid);
                report_actions.extend(self.asset_report.show(ctx, &report, selected));
            }

            if let Some(cmd) = ui_ctx_ref.palette.lock().show(ctx) {
                run_command(&ui_ctx_ref, &mut self.dock, &cmd);
            }
            recovery_answer = self.recovery.show(ctx);
            preferences_answer = self.preferences.show(ctx);
        });
        if let Some(answer) = recovery_answer {
            self.apply_recovery_answer(answer);
        }
        if let Some(policy) = preferences_answer {
            self.apply_preferences(policy);
        }
        for action in report_actions {
            self.apply_report_action(action);
        }
        output
    }
}
