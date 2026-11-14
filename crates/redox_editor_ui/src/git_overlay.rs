use redox_editor_scene::{SceneEntityChange, WorldGitTracker};

/// Action requested from the git overlay (handled by the editor host).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitOverlayAction {
    /// No action.
    None,
    /// Refresh OFPA and scene git status.
    Refresh,
    /// Commit staged scene + `project.ron` (message in host state).
    Commit,
    /// Fast-forward pull only.
    Pull,
}

/// Panel showing version-control state for OFPA worlds and scene documents.
pub struct GitOverlay;

impl GitOverlay {
    /// Renders the overlay. Returns the action the host should perform.
    pub fn show(ui: &mut egui::Ui, tracker: &WorldGitTracker, commit_message: &mut String) -> GitOverlayAction {
        let mut action = GitOverlayAction::None;

        ui.horizontal(|ui| {
            ui.label(format!(
                "Git/OFPA: {} dirty · {}",
                tracker.dirty_count(),
                if tracker.is_available() {
                    if tracker.has_repo() {
                        "git2 ON"
                    } else {
                        "git2 (no repo)"
                    }
                } else {
                    "OFPA only"
                }
            ));
            if ui.button("Refresh").clicked() {
                action = GitOverlayAction::Refresh;
            }
        });

        ui.label(format!("Root: {}", tracker.world_root.display()));
        ui.label(tracker.summary());

        if let Some(scene_line) = tracker.scene_summary() {
            ui.separator();
            ui.label("Scene document");
            if let Some(status) = tracker.scene_status.as_ref() {
                ui.label(format!(
                    "File: {} · {}",
                    status.path.display(),
                    scene_line
                ));
            } else {
                ui.label("No scene file tracked (set via project open)");
            }
            if !tracker.scene_diff.is_empty() {
                ui.label(format!("Object diff ({} changes)", tracker.scene_diff.len()));
                for change in tracker.scene_diff.iter().take(16) {
                    ui.label(change_label(change));
                }
                let rest = tracker.scene_diff.len().saturating_sub(16);
                if rest > 0 {
                    ui.label(format!("… +{rest} more"));
                }
            }
            ui.horizontal(|ui| {
                ui.text_edit_singleline(commit_message);
                if ui.button("Commit").clicked() {
                    action = GitOverlayAction::Commit;
                }
                if ui.button("Pull (ff-only)").clicked() {
                    action = GitOverlayAction::Pull;
                }
            });
        }

        ui.separator();
        let dirty = tracker.dirty_actor_ids();
        if dirty.is_empty() {
            ui.label("No dirty actors");
        } else {
            ui.label(format!("Dirty actors ({})", dirty.len()));
            let snippet: Vec<&str> = dirty.into_iter().take(12).collect();
            for id in &snippet {
                let badge = tracker.status_for(id).badge();
                ui.label(format!("{badge} {id}"));
            }
            let remaining = tracker.dirty_count().saturating_sub(snippet.len());
            if remaining > 0 {
                ui.label(format!("… +{remaining} more"));
            }
        }

        ui.label("Revert: Outliner → actor → Revert (restores disk / git checkout)");
        action
    }
}

fn change_label(change: &SceneEntityChange) -> String {
    match change {
        SceneEntityChange::Added { guid } => format!("+ entity {guid}"),
        SceneEntityChange::Removed { guid } => format!("− entity {guid}"),
        SceneEntityChange::Changed {
            guid,
            changed_components,
        } => format!("~ entity {guid} ({})", changed_components.join(", ")),
    }
}