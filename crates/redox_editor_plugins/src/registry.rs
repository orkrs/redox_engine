//! Accumulator for one plugin's registrations before they are applied to the editor.

use crate::validation::EditorValidationRule;
use redox_editor_assets::AssetImporter;
use redox_editor_core::EditorCommand;
use std::sync::Arc;

/// Metadata for a dock panel a plugin exposes (drawing stays in the UI crate).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelRegistration {
    /// Stable panel id, for example `hello`.
    pub id: String,
    /// Tab title shown in the dock.
    pub title: String,
}

/// Collects commands, panels, importers and validation rules from one plugin.
#[derive(Default)]
pub struct PluginRegistry {
    commands: Vec<Arc<dyn EditorCommand>>,
    panels: Vec<PanelRegistration>,
    importers: Vec<Arc<dyn AssetImporter>>,
    rules: Vec<Arc<dyn EditorValidationRule>>,
}

impl PluginRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a command.
    pub fn register_command(&mut self, command: impl EditorCommand + 'static) {
        self.commands.push(Arc::new(command));
    }

    /// Registers a dock panel by id and title.
    pub fn register_panel(&mut self, id: impl Into<String>, title: impl Into<String>) {
        self.panels.push(PanelRegistration {
            id: id.into(),
            title: title.into(),
        });
    }

    /// Registers an asset importer.
    pub fn register_importer(&mut self, importer: Arc<dyn AssetImporter>) {
        self.importers.push(importer);
    }

    /// Registers a validation rule.
    pub fn register_validation_rule(&mut self, rule: Arc<dyn EditorValidationRule>) {
        self.rules.push(rule);
    }

    /// Commands gathered so far.
    pub fn commands(&self) -> &[Arc<dyn EditorCommand>] {
        &self.commands
    }

    /// Panel registrations gathered so far.
    pub fn panels(&self) -> &[PanelRegistration] {
        &self.panels
    }

    /// Importers gathered so far.
    pub fn importers(&self) -> &[Arc<dyn AssetImporter>] {
        &self.importers
    }

    /// Validation rules gathered so far.
    pub fn validation_rules(&self) -> &[Arc<dyn EditorValidationRule>] {
        &self.rules
    }
}