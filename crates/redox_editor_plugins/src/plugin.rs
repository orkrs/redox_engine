//! Plugin trait and built-in catalog.

use crate::registry::PluginRegistry;
use crate::PLUGIN_API_VERSION;

/// One in-process editor plugin.
pub trait EditorPlugin: Send + Sync {
    /// Unique name, stored in `project.ron` `plugins`.
    fn name(&self) -> &str;
    /// API version the plugin was built against.
    fn api_version(&self) -> u32 {
        PLUGIN_API_VERSION
    }
    /// Registers panels, commands and other hooks.
    fn register(&self, registry: &mut PluginRegistry);
}

/// Names of plugins shipped inside the editor binary.
pub const BUILTIN_PLUGIN_NAMES: &[&str] = &["hello"];