use redox_editor_core::BrushSettings;

/// Paint-brush UI + settings (B to toggle).
pub struct BrushPanel {
    /// Whether the brush tool is on.
    pub active: bool,
    /// Brush parameters.
    pub settings: BrushSettings,
    /// Kind of actor the brush places.
    pub kind: String,
}

impl Default for BrushPanel {
    fn default() -> Self {
        Self {
            active: false,
            settings: BrushSettings::default(),
            kind: "prop".into(),
        }
    }
}

impl BrushPanel {
    /// Draws the brush checkbox and, when active, its sliders.
    pub fn show_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.active, "Brush (B)");
        if self.active {
            ui.add(egui::Slider::new(&mut self.settings.radius, 0.5..=12.0).text("R"));
            ui.add(egui::Slider::new(&mut self.settings.density, 1.0..=24.0).text("Dens"));
            ui.text_edit_singleline(&mut self.kind);
        }
    }
}
