//! Built-in plugins compiled into the editor.

mod hello;

pub use hello::{HelloPlugin, HELLO_COMMAND_ID, HELLO_PANEL_ID, HELLO_PLUGIN_NAME};

use crate::plugin::{EditorPlugin, BUILTIN_PLUGIN_NAMES};
use std::sync::Arc;

/// Every built-in plugin instance.
pub fn builtin_plugins() -> Vec<Arc<dyn EditorPlugin>> {
    vec![Arc::new(HelloPlugin)]
}

/// Whether `name` is a built-in plugin id.
pub fn is_builtin(name: &str) -> bool {
    BUILTIN_PLUGIN_NAMES.contains(&name)
}