//! Plugin manifest and API types of RedOx Studio. Skeleton: there is no plugin loader yet
//! (plugins are deferred, see `docs/engineering/EDITOR_PLAN.md` D6).

#![warn(missing_docs)]

/// Version of the plugin API a manifest was written for.
pub const PLUGIN_API_VERSION: u32 = 1;

/// Description of a plugin as listed in a project's manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginManifest {
    /// Unique plugin name.
    pub name: String,
    /// Plugin version text.
    pub version: String,
    /// Plugin API version the plugin was built for.
    pub api_version: u32,
}

impl PluginManifest {
    /// Creates a manifest for the current API version.
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            api_version: PLUGIN_API_VERSION,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_carries_the_current_api_version() {
        let m = PluginManifest::new("demo", "0.1.0");
        assert_eq!(m.api_version, PLUGIN_API_VERSION);
        assert_eq!(m.name, "demo");
    }
}
