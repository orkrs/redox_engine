use redox_ecs::{Entity, World};
use redox_editor_core::{Selection, WorldGitTracker, WorldSession};
use redox_math::Transform;
use redox_scene::{PartitionCell, StreamedActor};

pub struct OutlinerPanel {
    pub filter: String,
}

impl Default for OutlinerPanel {
    fn default() -> Self {
        Self {
            filter: String::new(),
        }
    }
}

pub enum OutlinerAction {
    LoadCell(PartitionCell),
    UnloadCell(PartitionCell),
    FocusActor { cell: PartitionCell, id: String },
    RevertActor { cell: PartitionCell, id: String },
}

impl OutlinerPanel {
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        world: &World,
        selection: &mut Selection,
        session: Option<&WorldSession>,
        git: Option<&WorldGitTracker>,
    ) -> Vec<OutlinerAction> {
        let mut actions = Vec::new();
        ui.horizontal(|ui| {
            ui.label("🔍");
            ui.text_edit_singleline(&mut self.filter);
        });
        ui.separator();

        let filter = self.filter.to_lowercase();

        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(session) = session {
                let name = &session.manifest.name;
                let viewer = {
                    // Approximate from selection or origin
                    let pos = selection
                        .primary
                        .and_then(|e| world.get_component::<Transform>(e).map(|t| t.translation))
                        .unwrap_or(redox_math::Vec3::new(56.0, 0.0, 56.0));
                    session.partition.cell_at(pos)
                };
                ui.collapsing(format!("🌐 {name}"), |ui| {
                    for cell in session.list_cells() {
                        let emoji = session.cell_status_emoji(cell, viewer);
                        let loaded = session.is_loaded(cell);
                        let header = format!("{emoji} ⊞ ({}, {})", cell.x, cell.y);
                        ui.collapsing(header, |ui| {
                            ui.horizontal(|ui| {
                                if loaded {
                                    if ui.small_button("Unload").clicked() {
                                        actions.push(OutlinerAction::UnloadCell(cell));
                                    }
                                } else if ui.small_button("Load").clicked() {
                                    actions.push(OutlinerAction::LoadCell(cell));
                                }
                            });
                            let actors = session.list_cell_actors(cell);
                            for actor in actors {
                                if !filter.is_empty()
                                    && !actor.id.to_lowercase().contains(&filter)
                                    && !actor.kind.to_lowercase().contains(&filter)
                                {
                                    continue;
                                }
                                let badge =
                                    git.map(|g| g.status_for(&actor.id).badge()).unwrap_or("🔵");
                                let label = format!("{badge} {} ({})", actor.id, actor.kind);
                                // Find live entity
                                let live = find_streamed(world, &actor.id);
                                let selected = live.is_some_and(|e| selection.primary == Some(e));
                                ui.horizontal(|ui| {
                                    if ui.selectable_label(selected, &label).clicked() {
                                        if let Some(e) = live {
                                            selection.set_primary(Some(e));
                                        }
                                        actions.push(OutlinerAction::FocusActor {
                                            cell,
                                            id: actor.id.clone(),
                                        });
                                    }
                                    if git
                                        .map(|g| {
                                            matches!(
                                                g.status_for(&actor.id),
                                                redox_editor_core::ActorGitStatus::Modified { .. }
                                            )
                                        })
                                        .unwrap_or(false)
                                        && ui.small_button("Revert").clicked()
                                    {
                                        actions.push(OutlinerAction::RevertActor {
                                            cell,
                                            id: actor.id.clone(),
                                        });
                                    }
                                });
                            }
                        });
                    }
                });
            } else {
                ui.collapsing("🌐 Scene (no world)", |ui| {
                    let mut entities: Vec<Entity> = world.all_entities().collect();
                    entities.sort_by_key(|e| e.id());
                    for e in entities {
                        let label = if let Some(t) = world.get_component::<Transform>(e) {
                            format!(
                                "Entity #{} ({:.0},{:.0},{:.0})",
                                e.id(),
                                t.translation.x,
                                t.translation.y,
                                t.translation.z
                            )
                        } else {
                            format!("Entity #{}", e.id())
                        };
                        if !filter.is_empty() && !label.to_lowercase().contains(&filter) {
                            continue;
                        }
                        let selected = selection.primary == Some(e);
                        if ui.selectable_label(selected, label).clicked() {
                            selection.set_primary(Some(e));
                        }
                    }
                });
            }
            ui.separator();
            ui.label("Grid: 🟩 loaded 🟨 near ⬛ unloaded · 🟢 added 🟡 dirty 🔵 clean");
        });
        actions
    }
}

fn find_streamed(world: &World, id: &str) -> Option<Entity> {
    world.all_entities().find(|e| {
        world
            .get_component::<StreamedActor>(*e)
            .is_some_and(|a| a.id == id)
    })
}
