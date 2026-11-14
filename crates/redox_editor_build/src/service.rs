//! Build profiles and cook orchestration for the editor and CLI.

use std::path::{Path, PathBuf};

use redox_asset::AssetRegistry;
use redox_ecs::World;
use redox_editor_core::project::{BuildProfile, ProjectContext, ProjectManifest};
use redox_editor_scene::{SceneDocument, register_scene_components};
use redox_reflect::TypeRegistry;

use crate::cook::{CookReport, cook_pack};
use crate::plan::{build_cook_plan, seed_scene_path};
use crate::snapshot::BuildSnapshot;

/// Build and cook errors (user-facing).
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    /// Project files could not be read.
    #[error(transparent)]
    Project(#[from] redox_editor_core::project::ProjectError),
    /// A logical cook failure.
    #[error("{0}")]
    Failed(String),
}

/// Lists profiles from a manifest, or the default `release` profile when the list is empty.
pub fn list_profiles(manifest: &ProjectManifest) -> Vec<BuildProfile> {
    if manifest.build_profiles.is_empty() {
        vec![BuildProfile::release_default()]
    } else {
        manifest.build_profiles.clone()
    }
}

/// Resolves a named profile, or the first profile / default when `name` is `None`.
pub fn resolve_profile<'a>(
    manifest: &'a ProjectManifest,
    name: Option<&str>,
) -> Result<&'a BuildProfile, BuildError> {
    if manifest.build_profiles.is_empty() {
        return Err(BuildError::Failed(
            "no build_profiles in manifest; add one or rely on the implicit release default via BuildService::list_profiles".into(),
        ));
    }
    if let Some(name) = name {
        manifest
            .build_profiles
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| BuildError::Failed(format!("unknown build profile {name}")))
    } else {
        manifest
            .build_profiles
            .first()
            .ok_or_else(|| BuildError::Failed("no build profiles".into()))
    }
}

/// Resolves a profile for cook, injecting the default when the manifest list is empty.
pub fn resolve_profile_for_cook(
    manifest: &ProjectManifest,
    name: Option<&str>,
) -> Result<BuildProfile, BuildError> {
    if manifest.build_profiles.is_empty() {
        let mut profile = BuildProfile::release_default();
        if let Some(name) = name {
            profile.name = name.to_string();
        }
        return Ok(profile);
    }
    let picked = resolve_profile(
        manifest,
        name.or_else(|| manifest.build_profiles.first().map(|p| p.name.as_str())),
    )?;
    Ok(picked.clone())
}

/// Orchestrates profile listing, planning and cooking.
#[derive(Debug, Default, Clone, Copy)]
pub struct BuildService;

impl BuildService {
    /// Profiles stored in the manifest, or the implicit default list.
    pub fn list_profiles(&self, manifest: &ProjectManifest) -> Vec<BuildProfile> {
        list_profiles(manifest)
    }

    /// Active profile by name; when `name` is `None`, the first manifest profile or default.
    pub fn resolve_active(
        &self,
        manifest: &ProjectManifest,
        name: Option<&str>,
    ) -> Result<BuildProfile, BuildError> {
        resolve_profile_for_cook(manifest, name)
    }

    /// Dependency closure for `profile`.
    pub fn plan(
        &self,
        project: &ProjectContext,
        profile: &BuildProfile,
    ) -> Result<crate::plan::CookPlan, BuildError> {
        let root = project.root();
        let assets_root = root.join(project.manifest().assets_root.as_str());
        let (registry, _) =
            AssetRegistry::scan(&assets_root).map_err(|e| BuildError::Failed(e.to_string()))?;
        let seed =
            seed_scene_path(root, project.manifest(), profile).map_err(BuildError::Failed)?;
        build_cook_plan(root, &registry, profile, &seed).map_err(BuildError::Failed)
    }

    /// Packs `profile` into `out_pak` and writes `report_json`; strips editor-only entities in the
    /// reported runtime entity count.
    pub fn cook(
        &self,
        project: &ProjectContext,
        profile: &BuildProfile,
        out_pak: &Path,
        report_json: &Path,
    ) -> Result<CookReport, BuildError> {
        let root = project.root();
        let assets_root = root.join(project.manifest().assets_root.as_str());
        let (registry, _) =
            AssetRegistry::scan(&assets_root).map_err(|e| BuildError::Failed(e.to_string()))?;
        let seed =
            seed_scene_path(root, project.manifest(), profile).map_err(BuildError::Failed)?;
        let plan = build_cook_plan(root, &registry, profile, &seed).map_err(BuildError::Failed)?;

        let runtime_entities = snapshot_runtime_entities(root, project.manifest(), &seed, profile);

        cook_pack(
            &registry,
            &plan,
            &profile.name,
            out_pak,
            report_json,
            profile.compress,
            runtime_entities,
            None,
            None,
        )
        .map_err(BuildError::Failed)
    }
}

fn snapshot_runtime_entities(
    _root: &Path,
    _manifest: &ProjectManifest,
    seed_scene: &Path,
    _profile: &BuildProfile,
) -> Option<usize> {
    let mut registry = TypeRegistry::new();
    register_scene_components(&mut registry);
    let mut world = World::new();
    let assets_root = seed_scene
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("assets"));
    let Ok((_doc, _)) =
        SceneDocument::open_with_assets(seed_scene, &registry, &mut world, assets_root.as_deref())
    else {
        return None;
    };
    Some(BuildSnapshot::from_world(&registry, &world).entity_count())
}
