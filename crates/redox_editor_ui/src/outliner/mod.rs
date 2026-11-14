//! Outliner panel: scene hierarchy or OFPA world cells.

mod scene;
mod world;

use std::collections::HashSet;

use redox_ecs::World;
use redox_editor_core::undo::UndoStack;
use redox_editor_core::{EditorContext, Selection};
use redox_editor_scene::guid_index::GuidIndex;
use redox_editor_scene::{WorldGitTracker, WorldSession};
use redox_reflect::TypeRegistry;
use redox_scene::{EntityGuid, PartitionCell};

pub use scene::OutlinerDragPayload;

/// Panel listing scene entities or world cells.
#[derive(Default)]
pub struct OutlinerPanel {
    /// Text filter for names.
    pub filter: String,
    /// Filter by reflected component type name.
    pub component_filter: String,
    /// When set, only entities with validation errors are listed.
    pub errors_only: bool,
    /// Expanded folder guids in scene mode.
    pub expanded: HashSet<EntityGuid>,
    /// Shift-range anchor guid.
    pub range_anchor: Option<EntityGuid>,
    /// Guids flagged by validation (may be empty until B7).
    pub validation_errors: HashSet<EntityGuid>,
    /// Number of warnings already logged this session.
    pub(crate) warned_count: usize,
    /// Rows in the last flat list (metrics).
    pub flat_len: usize,
    /// Rows drawn last frame (virtual list).
    pub last_rows_drawn: usize,
}

/// Something the user asked the outliner to do.
pub enum OutlinerAction {
    /// Load a cell.
    LoadCell(PartitionCell),
    /// Unload a cell.
    UnloadCell(PartitionCell),
    /// Move the camera to an actor.
    FocusActor {
        /// Cell that contains the actor.
        cell: PartitionCell,
        /// Id of the actor.
        id: String,
    },
    /// Restore an actor file from version control.
    RevertActor {
        /// Cell that contains the actor.
        cell: PartitionCell,
        /// Id of the actor.
        id: String,
    },
    /// Focus the editor camera on a scene entity.
    FocusScene(EntityGuid),
}

impl OutlinerPanel {
    /// Draws the outliner and returns the actions requested this frame.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        world: &World,
        registry: &TypeRegistry,
        selection: &mut Selection,
        undo: &mut UndoStack,
        core: &EditorContext,
        scene_mode: bool,
        session: Option<&WorldSession>,
        git: Option<&WorldGitTracker>,
    ) -> Vec<OutlinerAction> {
        ui.horizontal(|ui| {
            ui.label("🔍");
            ui.text_edit_singleline(&mut self.filter);
            if scene_mode {
                ui.label("comp");
                ui.text_edit_singleline(&mut self.component_filter);
                ui.checkbox(&mut self.errors_only, "errors");
            }
        });
        ui.separator();

        if scene_mode {
            let mut index = GuidIndex::new();
            index.rebuild(world);
            scene::show_scene(ui, self, world, registry, selection, undo, core, &index)
        } else if let Some(session) = session {
            world::show_world(ui, &self.filter, world, selection, session, git)
        } else {
            ui.label("No level loaded.");
            Vec::new()
        }
    }
}
