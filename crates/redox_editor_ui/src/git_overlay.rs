use redox_editor_core::WorldGitTracker;

/// Panel showing the version-control state of the world.
pub struct GitOverlay;

impl GitOverlay {
    /// Returns true if Refresh was clicked (caller should set `git_refresh` status).
    pub fn show(ui: &mut egui::Ui, tracker: &WorldGitTracker) -> bool {
        let mut refresh = false;

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
                refresh = true;
            }
        });

        ui.label(format!("Root: {}", tracker.world_root.display()));
        ui.label(tracker.summary());

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
        refresh
    }
}
