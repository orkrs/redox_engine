/// A command that can be run from the palette.
pub struct Command {
    /// Identifier handed to the host when the command is picked.
    pub id: String,
    /// Text shown in the list.
    pub label: String,
}

/// Searchable list of editor commands.
pub struct CommandPalette {
    /// Whether the palette is shown.
    pub open: bool,
    /// Text typed into the search box.
    pub query: String,
    /// All available commands.
    pub commands: Vec<Command>,
}

impl Default for CommandPalette {
    fn default() -> Self {
        let commands = vec![
            Command {
                id: "bake_meshlets".into(),
                label: "> Bake Meshlets".into(),
            },
            Command {
                id: "bake_report".into(),
                label: "> Bake Report".into(),
            },
            Command {
                id: "spawn_agent".into(),
                label: "> Spawn Demo Agent".into(),
            },
            Command {
                id: "align_surface".into(),
                label: "> Align to Surface".into(),
            },
            Command {
                id: "focus".into(),
                label: "@ Focus Entity".into(),
            },
            Command {
                id: "create_entity".into(),
                label: "> Create Empty Entity".into(),
            },
            Command {
                id: "duplicate".into(),
                label: "> Duplicate Entity".into(),
            },
            Command {
                id: "delete".into(),
                label: "> Delete Entity".into(),
            },
            Command {
                id: "reset_layout".into(),
                label: "> Reset Layout".into(),
            },
            Command {
                id: "pie_play".into(),
                label: "> Play (PIE)".into(),
            },
            Command {
                id: "pie_stop".into(),
                label: "> Stop (PIE)".into(),
            },
            Command {
                id: "shading_lit".into(),
                label: "> Shading: Lit".into(),
            },
            Command {
                id: "shading_unlit".into(),
                label: "> Shading: Unlit".into(),
            },
            Command {
                id: "shading_normals".into(),
                label: "> Shading: Normals".into(),
            },
            Command {
                id: "shading_wire".into(),
                label: "> Shading: Wireframe".into(),
            },
        ];
        Self {
            open: false,
            query: String::new(),
            commands,
        }
    }
}

impl CommandPalette {
    /// Opens or closes the palette; opening clears the query.
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.query.clear();
        }
    }

    /// Returns selected command id when user confirms.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<String> {
        if !self.open {
            return None;
        }
        let mut result = None;
        let mut close = false;
        egui::Window::new("Command Palette")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 80.0])
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.label("Ctrl+P · @entity  #asset  >command");
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .desired_width(f32::INFINITY)
                        .hint_text("> Bake Meshlets"),
                );
                resp.request_focus();

                let q = self.query.to_lowercase();
                let filtered: Vec<_> = self
                    .commands
                    .iter()
                    .filter(|c| {
                        q.is_empty() || c.label.to_lowercase().contains(&q) || c.id.contains(&q)
                    })
                    .collect();

                egui::ScrollArea::vertical()
                    .max_height(240.0)
                    .show(ui, |ui| {
                        for cmd in filtered {
                            if ui.selectable_label(false, &cmd.label).clicked() {
                                result = Some(cmd.id.clone());
                                close = true;
                            }
                        }
                    });

                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
                if ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && let Some(cmd) = self.commands.iter().find(|c| {
                        q.is_empty() || c.label.to_lowercase().contains(&q) || c.id.contains(&q)
                    })
                {
                    result = Some(cmd.id.clone());
                    close = true;
                }
            });
        if close {
            self.open = false;
        }
        result
    }
}
