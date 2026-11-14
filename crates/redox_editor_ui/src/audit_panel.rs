//! Asset audit panel: orphans, duplicates, size map export (EDT-C2 steps 2–4).

use redox_asset::AssetRegistry;
use redox_editor_assets::{
    AuditConfig, export_size_map_csv, export_size_map_json, find_duplicates, find_orphans,
    run_audit,
};

/// Audit UI shell.
pub struct AuditPanel {
    /// Orphan force-keep list entered as guid text.
    pub force_keep_text: String,
    /// Last CSV export written to the clipboard.
    pub last_csv: String,
    /// Last JSON export written to the clipboard.
    pub last_json: String,
}

impl AuditPanel {
    /// Creates the panel.
    pub fn new() -> Self {
        Self {
            force_keep_text: String::new(),
            last_csv: String::new(),
            last_json: String::new(),
        }
    }

    /// Renders audit controls.
    pub fn show(&mut self, ui: &mut egui::Ui, registry: &AssetRegistry) {
        let config = self.build_config(registry);
        ui.heading("Orphans");
        for path in find_orphans(registry, &config) {
            ui.label(path);
        }
        ui.heading("Duplicates");
        for group in find_duplicates(registry) {
            ui.label(format!(
                "hash {:x} ({} files)",
                group.hash,
                group.members.len()
            ));
            for (_, path) in &group.members {
                ui.label(format!("  {path}"));
            }
        }
        ui.heading("Size map export");
        let report = run_audit(registry, &config);
        if ui.button("Copy CSV").clicked() {
            self.last_csv = export_size_map_csv(&report.size_map);
            ui.ctx().copy_text(self.last_csv.clone());
        }
        if ui.button("Copy JSON").clicked() {
            self.last_json = export_size_map_json(&report.size_map);
            ui.ctx().copy_text(self.last_json.clone());
        }
    }

    fn build_config(&self, registry: &AssetRegistry) -> AuditConfig {
        let mut config = AuditConfig::default();
        for token in self.force_keep_text.split_whitespace() {
            if let Ok(guid) = token.parse::<redox_asset::AssetGuid>() {
                config.force_keep.insert(guid);
            } else if let Ok(value) = u128::from_str_radix(token, 16) {
                config.force_keep.insert(redox_asset::AssetGuid(value));
            }
        }
        let _ = registry;
        config
    }
}

impl Default for AuditPanel {
    fn default() -> Self {
        Self::new()
    }
}
