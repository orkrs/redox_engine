//! Reference viewer panel (EDT-C2 step 1 / step 4).

use redox_asset::{AssetGuid, AssetRegistry};
use redox_editor_assets::{ReferenceIndex, ReferenceViewerModel};

/// Shows incoming and outgoing references for the selected asset.
pub struct ReferenceViewerPanel {
    /// Viewer model.
    pub model: ReferenceViewerModel,
    /// Optional selected guid from the browser.
    selected: Option<AssetGuid>,
}

impl ReferenceViewerPanel {
    /// Empty panel.
    pub fn new() -> Self {
        Self {
            model: ReferenceViewerModel::new(),
            selected: None,
        }
    }

    /// Sets the asset shown in the viewer.
    pub fn select(
        &mut self,
        guid: AssetGuid,
        registry: &AssetRegistry,
        index: &ReferenceIndex,
    ) {
        self.selected = Some(guid);
        self.model.select(guid, registry, index);
    }

    /// Renders the viewer.
    pub fn show(&self, ui: &mut egui::Ui) {
        let view = self.model.view();
        ui.label(format!(
            "Asset {}",
            view.path.as_deref().unwrap_or("<unknown>")
        ));
        ui.heading("Incoming (scenes)");
        if view.incoming.is_empty() {
            ui.label("No scene refers to this asset.");
        }
        for incoming in &view.incoming {
            ui.label(format!(
                "{} · entity {:?} · {}",
                incoming.scene,
                incoming.entity,
                incoming.component
            ));
        }
        ui.heading("Outgoing");
        if view.outgoing.is_empty() {
            ui.label("No outgoing references indexed.");
        }
        for outgoing in &view.outgoing {
            ui.label(format!(
                "{} · entity {:?} → guid {:?} path {:?}",
                outgoing.component, outgoing.entity, outgoing.guid, outgoing.path
            ));
        }
    }
}

impl Default for ReferenceViewerPanel {
    fn default() -> Self {
        Self::new()
    }
}