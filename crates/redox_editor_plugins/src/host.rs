//! Loads in-process plugins and applies their registrations safely.

use crate::PLUGIN_API_VERSION;
use crate::builtin::builtin_plugins;
use crate::plugin::EditorPlugin;
use crate::registry::{PanelRegistration, PluginRegistry};
use crate::validation::EditorValidationRule;
use redox_editor_assets::AssetImporter;
use redox_editor_core::log::EditorLog;
use redox_editor_core::{CommandRegistry, EditorCommand, EditorContext};
use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

/// Result of trying to enable one plugin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginLoadError {
    /// No plugin with this name is known.
    Unknown(String),
    /// The plugin targets a different API version.
    IncompatibleApi {
        /// Plugin name.
        name: String,
        /// Version the plugin expects.
        found: u32,
        /// Version the host supports.
        expected: u32,
    },
    /// `register` panicked; the plugin was disabled (safe mode).
    PanicOnRegister {
        /// Plugin name.
        name: String,
    },
}

/// Loads built-in plugins and tracks enabled project plugins.
#[derive(Clone)]
pub struct PluginHost {
    builtins: Arc<Vec<Arc<dyn EditorPlugin>>>,
    enabled: HashSet<String>,
    disabled_by_safe_mode: HashSet<String>,
    loaded_command_ids: Vec<String>,
    command_owners: HashMap<String, String>,
    active_panels: Vec<PanelRegistration>,
    active_importers: Vec<Arc<dyn AssetImporter>>,
    active_rules: Vec<Arc<dyn EditorValidationRule>>,
    /// When true, show a banner that a plugin was disabled after a panic.
    pub safe_mode_active: bool,
    /// Last safe-mode message for the UI banner.
    pub safe_mode_message: Option<String>,
}

impl Default for PluginHost {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginHost {
    /// Host with every built-in plugin known but none enabled.
    pub fn new() -> Self {
        Self {
            builtins: Arc::new(builtin_plugins()),
            enabled: HashSet::new(),
            disabled_by_safe_mode: HashSet::new(),
            loaded_command_ids: Vec::new(),
            command_owners: HashMap::new(),
            active_panels: Vec::new(),
            active_importers: Vec::new(),
            active_rules: Vec::new(),
            safe_mode_active: false,
            safe_mode_message: None,
        }
    }

    /// Names of every built-in plugin.
    pub fn catalog_names(&self) -> Vec<String> {
        self.builtins.iter().map(|p| p.name().to_string()).collect()
    }

    /// Whether `name` is enabled for the current project.
    pub fn is_enabled(&self, name: &str) -> bool {
        self.enabled.contains(name) && !self.disabled_by_safe_mode.contains(name)
    }

    /// Enabled plugin names in stable order.
    pub fn enabled_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.enabled.iter().cloned().collect();
        names.sort();
        names
    }

    /// Panel tabs contributed by enabled plugins.
    pub fn active_panels(&self) -> &[PanelRegistration] {
        &self.active_panels
    }

    /// Importers contributed by enabled plugins.
    pub fn active_importers(&self) -> &[Arc<dyn AssetImporter>] {
        &self.active_importers
    }

    /// Validation rules contributed by enabled plugins.
    pub fn active_validation_rules(&self) -> &[Arc<dyn EditorValidationRule>] {
        &self.active_rules
    }

    /// Whether `id` was registered by an enabled plugin.
    pub fn owns_command(&self, id: &str) -> bool {
        self.command_owners.contains_key(id)
    }

    /// Clears safe-mode UI state (does not re-enable panicking plugins).
    pub fn dismiss_safe_mode_banner(&mut self) {
        self.safe_mode_active = false;
        self.safe_mode_message = None;
    }

    /// Enables plugins listed in `project.ron` and applies them to `commands`.
    pub fn apply_project_plugins(
        &mut self,
        requested: &[String],
        commands: &mut CommandRegistry,
        log: &EditorLog,
    ) {
        self.enabled = requested
            .iter()
            .filter(|name| self.find_plugin(name).is_some())
            .cloned()
            .collect();
        self.refresh_commands(commands, log);
    }

    /// Enables or disables one plugin and refreshes `commands`.
    pub fn set_enabled(
        &mut self,
        name: &str,
        enable: bool,
        commands: &mut CommandRegistry,
        log: &EditorLog,
    ) -> Result<(), PluginLoadError> {
        if enable {
            self.enable_one(name, log)?;
            self.disabled_by_safe_mode.remove(name);
        } else {
            self.enabled.remove(name);
        }
        self.refresh_commands(commands, log);
        Ok(())
    }

    /// Runs a plugin-aware command with panic isolation.
    pub fn run_command(
        &mut self,
        commands: &mut CommandRegistry,
        ctx: &EditorContext,
        id: &str,
    ) -> bool {
        let Some(command) = commands.get(id) else {
            ctx.log.warn(format!("Unknown command: {id}"));
            return false;
        };
        let owner = self.command_owners.get(id).cloned();
        let result = catch_unwind(AssertUnwindSafe(|| command.execute(ctx)));
        if result.is_err() {
            if let Some(plugin_name) = owner {
                self.enter_safe_mode_after_panic(&plugin_name, id, &ctx.log);
                self.refresh_commands(commands, &ctx.log);
            }
            return false;
        }
        true
    }

    fn enable_one(&mut self, name: &str, log: &EditorLog) -> Result<(), PluginLoadError> {
        let Some(plugin) = self.find_plugin(name) else {
            return Err(PluginLoadError::Unknown(name.to_string()));
        };
        if plugin.api_version() != PLUGIN_API_VERSION {
            log.warn(format!(
                "[plugins] skipped {name}: api {} != host {PLUGIN_API_VERSION}",
                plugin.api_version()
            ));
            return Err(PluginLoadError::IncompatibleApi {
                name: name.to_string(),
                found: plugin.api_version(),
                expected: PLUGIN_API_VERSION,
            });
        }
        self.enabled.insert(name.to_string());
        Ok(())
    }

    fn refresh_commands(&mut self, commands: &mut CommandRegistry, log: &EditorLog) {
        for id in self.loaded_command_ids.drain(..) {
            commands.unregister(&id);
            self.command_owners.remove(&id);
        }
        self.active_panels.clear();
        self.active_importers.clear();
        self.active_rules.clear();

        let mut enabled: Vec<String> = self.enabled.iter().cloned().collect();
        enabled.sort();

        for name in enabled {
            if self.disabled_by_safe_mode.contains(&name) {
                continue;
            }
            let Some(plugin) = self.find_plugin(&name) else {
                continue;
            };
            let mut registry = PluginRegistry::new();
            let register_ok =
                catch_unwind(AssertUnwindSafe(|| plugin.register(&mut registry))).is_ok();
            if !register_ok {
                self.disabled_by_safe_mode.insert(name.clone());
                self.safe_mode_active = true;
                self.safe_mode_message = Some(format!(
                    "Safe mode: plugin `{name}` was disabled after a panic during register."
                ));
                log.error(format!(
                    "[plugins] plugin {name} panicked while registering; it was disabled"
                ));
                continue;
            }
            for command in registry.commands() {
                let id = command.id().to_string();
                let wrapped = GuardedCommand {
                    inner: Arc::clone(command),
                };
                commands.register(wrapped);
                self.loaded_command_ids.push(id.clone());
                self.command_owners.insert(id, name.clone());
            }
            self.active_panels.extend(registry.panels().iter().cloned());
            self.active_importers
                .extend(registry.importers().iter().cloned());
            for rule in registry.validation_rules() {
                self.active_rules.push(Arc::clone(rule));
            }
        }
    }

    fn enter_safe_mode_after_panic(&mut self, plugin: &str, command_id: &str, log: &EditorLog) {
        self.disabled_by_safe_mode.insert(plugin.to_string());
        self.safe_mode_active = true;
        self.safe_mode_message = Some(format!(
            "Safe mode: plugin `{plugin}` was disabled after command `{command_id}` panicked."
        ));
        log.error(format!(
            "[plugins] plugin {plugin} panicked in command {command_id}; it was disabled"
        ));
    }

    fn find_plugin(&self, name: &str) -> Option<Arc<dyn EditorPlugin>> {
        self.builtins
            .iter()
            .find(|plugin| plugin.name() == name)
            .cloned()
    }
}

#[cfg(test)]
impl PluginHost {
    /// Adds plugins for unit tests.
    pub(crate) fn with_extra_builtins(mut self, extra: Vec<Arc<dyn EditorPlugin>>) -> Self {
        let mut list = (*self.builtins).clone();
        list.extend(extra);
        self.builtins = Arc::new(list);
        self
    }

    /// Whether safe mode disabled a plugin.
    pub(crate) fn is_disabled_by_safe_mode(&self, name: &str) -> bool {
        self.disabled_by_safe_mode.contains(name)
    }
}

struct GuardedCommand {
    inner: Arc<dyn EditorCommand>,
}

impl EditorCommand for GuardedCommand {
    fn id(&self) -> &str {
        self.inner.id()
    }

    fn label(&self) -> &str {
        self.inner.label()
    }

    fn execute(&self, ctx: &EditorContext) {
        self.inner.execute(ctx);
    }
}
