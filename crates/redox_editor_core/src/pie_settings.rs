//! PIE start scene and command-line style arguments (EDT-B6).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Overrides applied when PIE starts (stored in editor / project settings).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PieSettings {
    /// Scene path relative to the project root; when set, PIE loads this scene into the
    /// sandbox world instead of the document currently open in the editor.
    pub start_scene: Option<String>,
    /// Arguments exposed to game systems through [`PieArgs`].
    pub args: Vec<String>,
}

impl PieSettings {
    /// Merges project-local overrides from `.redox/pie_settings.ron` when the file exists.
    pub fn load_project_overrides(root: &Path) -> Self {
        let path = root.join(".redox").join("pie_settings.ron");
        match std::fs::read_to_string(&path) {
            Ok(text) => ron::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Resolved start scene path under `project_root`, if configured.
    pub fn start_scene_path(&self, project_root: &Path) -> Option<PathBuf> {
        self.start_scene.as_ref().map(|rel| project_root.join(rel))
    }
}

/// ECS resource: PIE command-line arguments available during Play.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PieArgs {
    /// Argument vector (--foo bar style tokens).
    pub args: Vec<String>,
}

impl PieArgs {
    /// Whether `token` appears in the argument list.
    pub fn has_flag(&self, token: &str) -> bool {
        self.args.iter().any(|a| a == token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn args_resource_exposes_configured_tokens() {
        let args = PieArgs {
            args: vec!["--godmode".into(), "level2".into()],
        };
        assert!(args.has_flag("--godmode"));
        assert!(!args.has_flag("--cheat"));
    }

    #[test]
    fn start_scene_resolves_against_project_root() {
        let settings = PieSettings {
            start_scene: Some("scenes/entry.scene.ron".into()),
            args: Vec::new(),
        };
        let root = Path::new("/proj");
        assert_eq!(
            settings.start_scene_path(root),
            Some(root.join("scenes/entry.scene.ron"))
        );
    }

    #[test]
    fn project_overrides_load_from_redox_folder() {
        let dir = std::env::temp_dir().join(format!("redox_pie_settings_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".redox")).expect("mkdir");
        let ron = r#"(start_scene: Some("custom.scene.ron"), args: ["--test"])"#;
        fs::write(dir.join(".redox/pie_settings.ron"), ron).expect("write");
        let loaded = PieSettings::load_project_overrides(&dir);
        assert_eq!(loaded.start_scene.as_deref(), Some("custom.scene.ron"));
        assert_eq!(loaded.args, vec!["--test"]);
        let _ = fs::remove_dir_all(&dir);
    }
}
