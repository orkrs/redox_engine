//! Headless project validate / sample load for CI (EDT-B8 step 3).

use std::path::Path;

use redox_ecs::World;
use redox_editor_build::validate_project;
use redox_editor_core::project::ProjectContext;
use redox_editor_scene::{SceneDocument, register_scene_components};
use redox_reflect::TypeRegistry;

/// Validates a project directory; returns a process exit code.
pub fn validate_project_root(root: &Path) -> u8 {
    match validate_project(root) {
        Ok(report) => report.exit_code(),
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}

/// Opens the start scene and optionally simulates `frames` update ticks (validate+load only;
/// GPU frame stepping is 📌 until CI has headless wgpu).
pub fn run_sample(root: &Path, frames: u32) -> u8 {
    if validate_project_root(root) != 0 {
        return 1;
    }
    let project = match ProjectContext::open(root) {
        Ok(p) => p,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let Some(scene_path) = project.start_scene_path() else {
        eprintln!("project has no start_scene");
        return 1;
    };
    let assets = root.join(project.manifest().assets_root.as_str());
    let mut registry = TypeRegistry::new();
    register_scene_components(&mut registry);
    let mut world = World::new();
    match SceneDocument::open_with_assets(&scene_path, &registry, &mut world, Some(&assets)) {
        Ok((_doc, report)) => {
            if report
                .warnings
                .iter()
                .any(|w| matches!(w, redox_editor_scene::LoadWarning::MissingAsset { .. }))
            {
                return 1;
            }
        }
        Err(err) => {
            eprintln!("scene load failed: {err}");
            return 1;
        }
    }
    if frames > 0 {
        eprintln!(
            "note: --run-sample --frames {frames} performs load+validate only (GPU stepping 📌)"
        );
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn sample_project_cli_path_exits_zero() {
        let root = std::env::temp_dir().join(format!("editor_sample_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let _ = redox_editor_core::project::ProjectContext::create(
            &root,
            "Sample",
            redox_editor_core::project_template::ProjectTemplate::ThreeDStarter,
        );
        assert_eq!(run_sample(&root, 0), 0);
        let _ = std::fs::remove_dir_all(&root);
    }
}
