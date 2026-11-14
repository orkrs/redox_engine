//! In-process plugin host wiring.

use super::EditorApp;
use redox_editor_plugins::HELLO_PLUGIN_NAME;

impl EditorApp {
    /// Applies `project.ron` plugins to the command registry and dock.
    pub(super) fn sync_project_plugins(&mut self) {
        let plugins = self
            .files
            .project()
            .map(|project| project.manifest().plugins.clone())
            .unwrap_or_default();
        self.plugin_host.apply_project_plugins(
            &plugins,
            &mut self.core.commands.lock(),
            &self.core.log,
        );
        let hello = self.plugin_host.is_enabled(HELLO_PLUGIN_NAME);
        self.dock.set_hello_plugin_tab(hello);
        self.ui_ctx
            .palette
            .lock()
            .add_registry_commands(self.core.commands.lock().entries());
    }

    /// Runs a command and keeps the UI command registry in sync.
    pub(super) fn run_editor_command(&mut self, id: &str) {
        crate::commands::run_command(
            &self.ui_ctx,
            &mut self.dock,
            Some(&mut self.plugin_host),
            &mut self.core.commands.lock(),
            self.settings.plugins.isolate_plugin_panics,
            id,
        );
    }
}
