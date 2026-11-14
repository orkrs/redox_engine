//! OFPA world partition outliner (unchanged behaviour from stage A).

use redox_ecs::{Entity, World};
use redox_editor_core::Selection;
use redox_editor_scene::{WorldGitTracker, WorldSession};
use redox_math::Transform;
use redox_scene::{PartitionCell, StreamedActor};

use super::OutlinerAction;

/// Draws the world / OFPA branch of the outliner.
pub fn show_world(
    ui: &mut egui::Ui,
    filter: &str,
    world: &World,
    selection: &mut Selection,
    session: &WorldSession,
    git: Option<&WorldGitTracker>,
) -> Vec<OutlinerAction> {
    let mut actions = Vec::new();
    let filter = filter.to_lowercase();

    let name = &session.manifest.name;
    let viewer = {
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
                    let badge = git.map(|g| g.status_for(&actor.id).badge()).unwrap_or("🔵");
                    let label = format!("{badge} {} ({})", actor.id, actor.kind);
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
                                    redox_editor_scene::ActorGitStatus::Modified { .. }
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
    ui.separator();
    ui.label("Grid: 🟩 loaded 🟨 near ⬛ unloaded · 🟢 added 🟡 dirty 🔵 clean");
    actions
}

fn find_streamed(world: &World, id: &str) -> Option<Entity> {
    world.all_entities().find(|e| {
        world
            .get_component::<StreamedActor>(*e)
            .is_some_and(|a| a.id == id)
    })
}
