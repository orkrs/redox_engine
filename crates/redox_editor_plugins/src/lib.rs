//! In-process editor plugin host for RedOx Studio.

#![warn(missing_docs)]

mod builtin;
mod host;
mod manifest;
mod native;
mod plugin;
mod registry;
mod validation;

pub use builtin::{
    HELLO_COMMAND_ID, HELLO_PANEL_ID, HELLO_PLUGIN_NAME, HelloPlugin, builtin_plugins, is_builtin,
};
pub use host::{PluginHost, PluginLoadError};
pub use manifest::{PLUGIN_API_VERSION, PluginManifest};
pub use native::{NATIVE_PLUGIN_ENTRY, NativePluginError, NativePluginHandle, load_native_plugin};
pub use plugin::{BUILTIN_PLUGIN_NAMES, EditorPlugin};
pub use registry::{PanelRegistration, PluginRegistry};
pub use validation::{EditorValidationRule, ValidationIssue, ValidationSeverity};

#[cfg(test)]
mod tests {
    use super::*;
    use redox_editor_core::{CommandRegistry, EditorCommand, EditorContext, EditorLog};
    use redox_reflect::TypeRegistry;
    use std::sync::Arc;

    struct MockCommand;

    impl EditorCommand for MockCommand {
        fn id(&self) -> &str {
            "mock.ping"
        }

        fn label(&self) -> &str {
            "> Mock"
        }

        fn execute(&self, _ctx: &EditorContext) {}
    }

    struct MockPlugin;

    impl EditorPlugin for MockPlugin {
        fn name(&self) -> &str {
            "mock"
        }

        fn register(&self, registry: &mut PluginRegistry) {
            registry.register_command(MockCommand);
        }
    }

    struct BadApiPlugin;

    impl EditorPlugin for BadApiPlugin {
        fn name(&self) -> &str {
            "bad_api"
        }

        fn api_version(&self) -> u32 {
            PLUGIN_API_VERSION + 1
        }

        fn register(&self, _registry: &mut PluginRegistry) {}
    }

    struct PanicRegisterPlugin;

    impl EditorPlugin for PanicRegisterPlugin {
        fn name(&self) -> &str {
            "panic_register"
        }

        fn register(&self, _registry: &mut PluginRegistry) {
            panic!("register boom");
        }
    }

    struct PanicRunCommand;

    impl EditorPlugin for PanicRunCommand {
        fn name(&self) -> &str {
            "panic_run"
        }

        fn register(&self, registry: &mut PluginRegistry) {
            registry.register_command(PanicRunCommandImpl);
        }
    }

    struct PanicRunCommandImpl;

    impl EditorCommand for PanicRunCommandImpl {
        fn id(&self) -> &str {
            "panic.run"
        }

        fn label(&self) -> &str {
            "> Panic"
        }

        fn execute(&self, _ctx: &EditorContext) {
            panic!("execute boom");
        }
    }

    fn test_host_with(extra: Vec<Arc<dyn EditorPlugin>>) -> PluginHost {
        PluginHost::new().with_extra_builtins(extra)
    }

    fn ctx() -> EditorContext {
        EditorContext::new(TypeRegistry::new())
    }

    #[test]
    fn mock_plugin_registers_command() {
        let mut host = test_host_with(vec![Arc::new(MockPlugin)]);
        let mut commands = CommandRegistry::new();
        let log = EditorLog::new();
        host.apply_project_plugins(&["mock".to_string()], &mut commands, &log);
        assert!(commands.contains("mock.ping"));
    }

    #[test]
    fn wrong_api_version_rejected() {
        let mut host = test_host_with(vec![Arc::new(BadApiPlugin)]);
        let mut commands = CommandRegistry::new();
        let log = EditorLog::new();
        let err = host
            .set_enabled("bad_api", true, &mut commands, &log)
            .expect_err("incompatible api");
        assert!(matches!(err, PluginLoadError::IncompatibleApi { .. }));
        assert!(!commands.contains("mock.ping"));
    }

    #[test]
    fn enable_disable_hello_plugin_changes_command_registry_ids() {
        let mut host = PluginHost::new();
        let mut commands = CommandRegistry::new();
        let log = EditorLog::new();
        host.set_enabled(HELLO_PLUGIN_NAME, true, &mut commands, &log)
            .expect("enable hello");
        assert!(commands.ids().contains(&HELLO_COMMAND_ID));
        host.set_enabled(HELLO_PLUGIN_NAME, false, &mut commands, &log)
            .expect("disable hello");
        assert!(!commands.contains(HELLO_COMMAND_ID));
    }

    #[test]
    fn panicking_plugin_does_not_crash_host_command_absent_after_disable() {
        let mut host = test_host_with(vec![Arc::new(PanicRunCommand)]);
        let mut commands = CommandRegistry::new();
        let log = EditorLog::new();
        host.apply_project_plugins(&["panic_run".to_string()], &mut commands, &log);
        assert!(commands.contains("panic.run"));
        let mut editor = ctx();
        assert!(!host.run_command(&mut commands, &editor, "panic.run"));
        assert!(!commands.contains("panic.run"));
        assert!(host.safe_mode_active);
    }

    #[test]
    fn panicking_register_disables_plugin_without_crashing_host() {
        let mut host = test_host_with(vec![Arc::new(PanicRegisterPlugin)]);
        let mut commands = CommandRegistry::new();
        let log = EditorLog::new();
        host.apply_project_plugins(&["panic_register".to_string()], &mut commands, &log);
        assert!(host.is_disabled_by_safe_mode("panic_register"));
        assert!(host.safe_mode_active);
    }
}
