//! Undo History panel: the steps of the undo stack, the current position, and jumps to any step.

use redox_editor_core::EditorContext;

/// Panel that lists the undo history and moves the document to any step of it.
pub struct UndoHistoryPanel;

impl UndoHistoryPanel {
    /// Draws the history of `core`: the initial state, the applied steps, then the undone steps
    /// that can still be redone (greyed out). Clicking a row moves the document to that position.
    pub fn show(ui: &mut egui::Ui, core: &EditorContext) {
        let (applied, undone, dirty) = {
            let undo = core.undo.lock();
            let applied: Vec<String> = undo.entries().iter().map(|e| e.label.clone()).collect();
            let undone: Vec<String> = undo
                .redo_entries()
                .iter()
                .rev()
                .map(|e| e.label.clone())
                .collect();
            (applied, undone, undo.is_dirty())
        };
        let current = applied.len();

        ui.horizontal(|ui| {
            ui.label(format!("{current} applied"));
            if dirty {
                ui.weak("unsaved changes");
            }
            let has_steps = !applied.is_empty() || !undone.is_empty();
            if ui
                .add_enabled(has_steps, egui::Button::new("Clear"))
                .clicked()
            {
                core.undo.lock().clear();
            }
        });
        ui.separator();

        let mut target = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            if ui
                .selectable_label(current == 0, "Initial state")
                .clicked()
            {
                target = Some(0);
            }
            for (index, label) in applied.iter().enumerate() {
                let position = index + 1;
                let text = format!("{position}. {label}");
                if ui.selectable_label(current == position, text).clicked() {
                    target = Some(position);
                }
            }
            for (index, label) in undone.iter().enumerate() {
                let position = current + index + 1;
                let text = egui::RichText::new(format!("{position}. {label}")).weak();
                if ui.selectable_label(false, text).clicked() {
                    target = Some(position);
                }
            }
        });

        if let Some(position) = target {
            Self::goto(core, position);
        }
    }

    /// Moves the document to `position`; a failure is written to the console.
    fn goto(core: &EditorContext, position: usize) {
        let error = {
            let mut world = core.world.lock();
            let registry = core.registry.lock();
            let mut undo = core.undo.lock();
            if !undo.goto(position, &mut world, &registry) {
                core.log
                    .warn(format!("[WARN] Undo history: position {position} not reached"));
            }
            undo.last_error.take()
        };
        if let Some(error) = error {
            core.log.error(format!("[ERROR] Undo history: {error}"));
        }
    }
}
