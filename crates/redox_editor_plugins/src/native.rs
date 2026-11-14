//! Native cdylib plugin loading (`editor-native-plugins` feature).

use std::path::Path;

/// C ABI entry exported by native editor plugins (`dlsym` name).
pub const NATIVE_PLUGIN_ENTRY: &[u8] = b"redox_editor_plugin_entry\0";

/// Handle to a loaded native plugin (not implemented — see report).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePluginHandle {
    /// Path passed to the loader.
    pub path: String,
}

/// Why native loading did not succeed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativePluginError {
    /// Feature disabled at compile time.
    FeatureDisabled,
    /// ABI loading is not implemented yet.
    NotImplemented(String),
}

impl std::fmt::Display for NativePluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FeatureDisabled => {
                write!(
                    f,
                    "native editor plugins are not enabled in this build (enable feature editor-native-plugins)"
                )
            }
            Self::NotImplemented(message) => {
                write!(f, "native plugin loading is not implemented: {message}")
            }
        }
    }
}

impl std::error::Error for NativePluginError {}

/// Loads a native plugin from `path`.
///
/// With `editor-native-plugins` enabled this is still a stub: dynamic loading is deferred until
/// the C ABI stabilizes (see `docs/EDITOR_PLUGINS.md`).
pub fn load_native_plugin(path: &Path) -> Result<NativePluginHandle, NativePluginError> {
    let _ = path;
    #[cfg(feature = "editor-native-plugins")]
    {
        Err(NativePluginError::NotImplemented(format!(
            "dlopen of {} is not wired yet; use in-process plugins",
            path.display()
        )))
    }
    #[cfg(not(feature = "editor-native-plugins"))]
    {
        let _ = NativePluginError::FeatureDisabled;
        Err(NativePluginError::FeatureDisabled)
    }
}
