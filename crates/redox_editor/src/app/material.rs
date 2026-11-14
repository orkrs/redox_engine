//! Drains asset-browser material open requests into `EditorUiCtx`.

use super::EditorApp;

impl EditorApp {
    /// Opens queued `.matgraph` files from the browser (double-click / Open).
    pub(super) fn sync_material_opens(&mut self) {
        let opens = self.ui_ctx.assets.lock().take_material_opens();
        for open in opens {
            match self.ui_ctx.open_material(open) {
                Ok(()) => {}
                Err(err) => self.ui_ctx.log(format!("[ERROR] Cannot open material: {err}")),
            }
        }
    }
}