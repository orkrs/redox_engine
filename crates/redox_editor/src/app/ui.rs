//! egui layout: menu bar, status bar, dock and command palette.

use super::EditorApp;
use crate::commands::{focus_selection, run_command};
use redox_editor_core::WorldSession;
use redox_math::Transform;

impl EditorApp {
    pub(super) fn build_ui(&mut self) -> egui::FullOutput {
        let raw_input = self.egui_winit.take_egui_input(&self.window);
        let ui_ctx_ref = self.ui_ctx.clone();
        self.egui_ctx.run(raw_input, |ctx| {
            // Ctrl+P palette
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::P)) {
                ui_ctx_ref.palette.lock().toggle();
            }

            egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.menu_button("File", |ui| {
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
                        if ui.button("Save Layout Ctrl+S").clicked() {
                            let _ = self
                                .dock
                                .save_to_file(std::path::Path::new("editor_layout.ron"));
                        }
                    });
                    ui.menu_button("Edit", |ui| {
                        if ui.button("Undo Ctrl+Z").clicked() {
                            let mut w = self.world.lock();
                            let r = self.registry.lock();
                            self.undo_stack.lock().undo(&mut w, &r);
                        }
                        if ui.button("Redo Ctrl+Y").clicked() {
                            let mut w = self.world.lock();
                            let r = self.registry.lock();
                            self.undo_stack.lock().redo(&mut w, &r);
                        }
                        if ui.button("Command Palette Ctrl+P").clicked() {
                            ui_ctx_ref.palette.lock().toggle();
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
                                    *ui_ctx_ref.status.lock() =
                                        format!("open_world:{}", root.display());
                                    ui.close_menu();
                                }
                            }
                        }
                        ui.separator();
                        if ui.button("Tick Streaming").clicked()
                            && let Some(session) = ui_ctx_ref.world_session.lock().as_mut()
                        {
                            let eye = self.camera.lock().eye;
                            session.tick(&mut self.world.lock(), eye);
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
                            *ui_ctx_ref.status.lock() = "git_refresh".into();
                            ui.close_menu();
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
                        let sel = self.selection.lock().primary.map(|e| e.id());
                        ui.label(format!("Sel:{:?}", sel));
                        ui.label(format!("Gizmo:{:?}", self.gizmo.lock().mode));
                    });
                });
            });

            egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let git = if ui_ctx_ref.world_session.lock().is_some() {
                        format!("🟢 {}", ui_ctx_ref.git.lock().summary())
                    } else {
                        "⚪ No world".into()
                    };
                    ui.label(git);
                    ui.separator();
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
                        .selection
                        .lock()
                        .primary
                        .and_then(|e| {
                            self.world.lock().get_component::<Transform>(e).map(|t| {
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
                    ui.label(format!("Undo: {}", self.undo_stack.lock().undo.len()));
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

            self.dock.show_with_ctx(
                ctx,
                &ui_ctx_ref,
                self.viewport_tex_id,
                Some(self.viewport_rect.clone()),
            );

            if let Some(cmd) = ui_ctx_ref.palette.lock().show(ctx) {
                run_command(&ui_ctx_ref, &mut self.dock, &cmd);
            }
        })
    }
}
