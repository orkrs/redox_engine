//! Project-wide validation for the cook CLI (EDT-B8).

use std::path::{Path, PathBuf};

use redox_asset::{AssetRegistry, RegistryError};
use redox_ecs::World;
use redox_editor_assets::ReferenceIndex;
use redox_editor_core::project::{ProjectContext, ProjectError, SCENE_SUFFIX};
use redox_reflect::TypeRegistry;

use super::{validate_level, ValidateLevelOptions, ValidationReport, ValidationSettings};
use crate::{Level, LevelError, register_scene_components};

/// Errors of [`validate_project`].
#[derive(Debug, thiserror::Error)]
pub enum ValidateProjectError {
    /// The project could not be opened.
    #[error(transparent)]
    Project(#[from] ProjectError),
    /// The assets root could not be scanned.
    #[error(transparent)]
    Registry(#[from] RegistryError),
    /// A scene file failed to load.
    #[error(transparent)]
    Level(#[from] LevelError),
}

/// Validates every scene under `root` and returns a merged report.
///
/// This is the public entry point for `redox_cook validate` (EDT-B8).
pub fn validate_project(root: impl AsRef<Path>) -> Result<ValidationReport, ValidateProjectError> {
    let root = root.as_ref();
    let project = ProjectContext::open(root)?;
    let assets_rel = project.manifest().assets_root.clone();
    let assets_root = root.join(&assets_rel);
    let (mut asset_registry, _) = AssetRegistry::scan(&assets_root)?;
    let mut reference_index = ReferenceIndex::default();
    let _ = reference_index.rebuild(&mut asset_registry);

    let mut registry = TypeRegistry::new();
    register_scene_components(&mut registry);

    let settings = ValidationSettings::default();
    let mut report = ValidationReport::default();

    let textures = super::rules::check_texture_mips(&super::rules::RuleContext {
        world: &World::new(),
        registry: &registry,
        assets_root: Some(assets_root.as_path()),
        asset_registry: Some(&asset_registry),
        reference_index: Some(&reference_index),
        scene_path: None,
        settings: &settings,
        load_report: None,
    });
    report.issues.extend(textures);

    for scene_path in project.scenes()? {
        let rel = scene_path
            .strip_prefix(root)
            .unwrap_or(scene_path.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        if !rel.ends_with(SCENE_SUFFIX) {
            continue;
        }
        let mut world = World::new();
        let (_, load_report) =
            Level::open_with_assets(&scene_path, &registry, &mut world, Some(&assets_root))?;
        let level_report = validate_level(
            &world,
            &registry,
            ValidateLevelOptions {
                assets_root: Some(&assets_root),
                asset_registry: Some(&asset_registry),
                reference_index: Some(&reference_index),
                scene_path: Some(&rel),
                settings,
                load_report: Some(&load_report),
            },
        );
        report.extend(level_report);
    }
    Ok(report)
}

/// Relative path of a scene for issue metadata.
pub fn scene_rel_path(root: &Path, scene: &Path) -> PathBuf {
    scene
        .strip_prefix(root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| scene.to_path_buf())
}