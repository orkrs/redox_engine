//! Demo in-process plugin: one command and a Hello dock tab.

use crate::plugin::EditorPlugin;
use crate::registry::PluginRegistry;
use redox_editor_core::{EditorCommand, EditorContext};

/// Stable name in `project.ron`.
pub const HELLO_PLUGIN_NAME: &str = "hello";

/// Id of the command the plugin registers.
pub const HELLO_COMMAND_ID: &str = "hello.greet";

/// Id of the dock panel the plugin registers.
pub const HELLO_PANEL_ID: &str = "hello";

/// In-process demo plugin.
pub struct HelloPlugin;

impl EditorPlugin for HelloPlugin {
    fn name(&self) -> &str {
        HELLO_PLUGIN_NAME
    }

    fn register(&self, registry: &mut PluginRegistry) {
        registry.register_command(HelloGreet);
        registry.register_panel(HELLO_PANEL_ID, "Hello Plugin");
    }
}

struct HelloGreet;

impl EditorCommand for HelloGreet {
    fn id(&self) -> &str {
        HELLO_COMMAND_ID
    }

    fn label(&self) -> &str {
        "> Hello from plugin"
    }

    fn execute(&self, ctx: &EditorContext) {
        ctx.log
            .info("HelloPlugin: editor plugin API is working".to_string());
    }
}