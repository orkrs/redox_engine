//! Editor preferences in `editor_settings.ron`, written atomically.

use crate::autosave::AutosavePolicy;
use crate::log::EditorLog;
use redox_scene::ofpa::write_atomic;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::Path;

/// File name of the settings, in the working directory next to `editor_layout.ron`.
pub const SETTINGS_FILE: &str = "editor_settings.ron";

/// Editor preferences. Missing fields take their defaults.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorSettings {
    /// Autosave switch, interval and number of kept snapshots.
    pub autosave: AutosavePolicy,
}

impl EditorSettings {
    /// Reads the settings file. Values outside their allowed range are clamped.
    pub fn load(path: &Path) -> io::Result<Self> {
        let text = fs::read_to_string(path)?;
        let settings: Self = ron::from_str(&text)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
        Ok(settings.clamped())
    }

    /// Reads the settings file. A missing file gives the defaults; an unreadable one is logged
    /// and also gives the defaults.
    pub fn load_or_default(path: &Path, log: &EditorLog) -> Self {
        match Self::load(path) {
            Ok(settings) => settings,
            Err(err) => {
                if err.kind() != io::ErrorKind::NotFound {
                    log.warn(format!(
                        "{} is unreadable, the defaults are used: {err}",
                        path.display()
                    ));
                }
                Self::default()
            }
        }
    }

    /// Writes the settings atomically.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|err| io::Error::other(err.to_string()))?;
        write_atomic(path, text.as_bytes())
    }

    /// Copy with every value brought into its allowed range.
    pub fn clamped(self) -> Self {
        Self {
            autosave: self.autosave.clamped(),
        }
    }
}
