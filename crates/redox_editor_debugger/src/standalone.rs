//! Standalone PIE launch planning (no subprocess spawn from the library).

use std::path::{Path, PathBuf};

/// How standalone play should be launched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StandalonePlan {
    /// Launch the editor binary in `--play` headless host mode with the project path.
    EditorPlayHost {
        /// Project directory passed to `--play`.
        project: PathBuf,
    },
    /// No launcher available; host should surface the reason to the user.
    Unavailable(String),
}

/// Builds a standalone launch plan. The debugger crate never shells out; the editor binary
/// or coordinator runs the returned plan.
pub fn plan_standalone(project_root: &Path, game_binary: Option<&Path>) -> StandalonePlan {
    if let Some(bin) = game_binary
        && bin.is_file()
    {
        return StandalonePlan::EditorPlayHost {
            project: project_root.to_path_buf(),
        };
    }
    // No dedicated game binary in-tree: documented `--play` host path (see EDT-B6 report 📌).
    if project_root.is_dir() {
        StandalonePlan::EditorPlayHost {
            project: project_root.to_path_buf(),
        }
    } else {
        StandalonePlan::Unavailable("project path is not a directory".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn plan_prefers_project_directory_for_play_host() {
        let dir = std::env::temp_dir().join(format!("redox_standalone_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");
        let plan = plan_standalone(&dir, None);
        assert!(matches!(plan, StandalonePlan::EditorPlayHost { .. }));
        let _ = fs::remove_dir_all(&dir);
    }
}