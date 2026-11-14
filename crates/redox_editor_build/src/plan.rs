//! Dependency closure for cook: which assets are included and why.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use redox_asset::{AssetGuid, AssetRegistry};
use redox_editor_assets::reference_index::scene_asset_refs;
use redox_editor_assets::{RefResolution, resolve_ref};
use redox_editor_core::project::BuildProfile;
use serde::{Deserialize, Serialize};

/// Why an asset is part of a cook plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InclusionReason {
    /// Referenced directly from the seed scene (transitive scene refs use the same reason).
    DirectFromScene,
    /// Pulled in because `parent` is in the plan (for example a texture inside a glTF).
    ImportedBy {
        /// Parent asset GUID (32 hex digits).
        guid: String,
    },
    /// Forced by the profile (`include_debug_assets` or explicit scene override).
    ProfileForced,
}

/// Assets selected for one cook, with explanations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CookPlan {
    /// GUIDs included in the pack, in deterministic order.
    pub assets: Vec<AssetGuid>,
    /// Why each GUID is present.
    pub reasons: HashMap<AssetGuid, InclusionReason>,
}

impl CookPlan {
    /// Whether `guid` is part of the plan.
    pub fn contains(&self, guid: AssetGuid) -> bool {
        self.reasons.contains_key(&guid)
    }
}

/// Builds a cook plan from the profile and a scanned assets registry.
pub fn build_cook_plan(
    project_root: &Path,
    registry: &AssetRegistry,
    profile: &BuildProfile,
    seed_scene: &Path,
) -> Result<CookPlan, String> {
    let mut reasons: HashMap<AssetGuid, InclusionReason> = HashMap::new();
    let mut queue: VecDeque<(AssetGuid, InclusionReason)> = VecDeque::new();

    let refs = scene_asset_refs(seed_scene).map_err(|e| e.to_string())?;
    for sref in refs {
        if let Some(guid) = resolve_to_guid(registry, &sref)? {
            queue.push_back((guid, InclusionReason::DirectFromScene));
        }
    }

    while let Some((guid, reason)) = queue.pop_front() {
        if reasons.contains_key(&guid) {
            continue;
        }
        reasons.insert(guid, reason.clone());
        let Some(path) = registry.path_of(guid) else {
            continue;
        };
        let abs = registry.root().join(path);
        for child in gltf_dependencies(registry, &abs, guid)? {
            if !reasons.contains_key(&child) {
                queue.push_back((
                    child,
                    InclusionReason::ImportedBy {
                        guid: format!("{guid:032x}"),
                    },
                ));
            }
        }
    }

    if profile.include_debug_assets {
        force_debug_tree(registry, &mut reasons);
    }

    let mut assets: Vec<AssetGuid> = reasons.keys().copied().collect();
    assets.sort_by_key(|guid| guid.0);

    Ok(CookPlan { assets, reasons })
}

fn resolve_to_guid(
    registry: &AssetRegistry,
    sref: &redox_editor_assets::SceneRef,
) -> Result<Option<AssetGuid>, String> {
    match resolve_ref(
        registry,
        sref.guid.as_deref(),
        sref.path.as_deref(),
    ) {
        RefResolution::Resolved { guid, .. } | RefResolution::Restored { guid, .. } => Ok(Some(guid)),
        RefResolution::Unset => Ok(None),
        RefResolution::Missing { .. } => Ok(None),
    }
}

fn gltf_dependencies(
    registry: &AssetRegistry,
    path: &Path,
    parent: AssetGuid,
) -> Result<Vec<AssetGuid>, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext != "glb" && ext != "gltf" {
        return Ok(Vec::new());
    }
    let Ok((document, _buffers, _images)) = gltf::import(path) else {
        return Ok(Vec::new());
    };
    let base = path.parent().unwrap_or_else(|| Path::new(""));
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for image in document.images() {
        let gltf::image::Source::Uri { uri, .. } = image.source() else {
            continue;
        };
        let resolved = base.join(uri);
        if let Ok(rel) = resolved.strip_prefix(registry.root()) {
            let rel = rel.to_string_lossy().replace('\\', "/");
            if let Some(guid) = registry.guid_of(&rel) {
                if seen.insert(guid) {
                    out.push(guid);
                }
            }
        }
    }
    let _ = parent;
    Ok(out)
}

fn force_debug_tree(registry: &AssetRegistry, reasons: &mut HashMap<AssetGuid, InclusionReason>) {
    for (path, guid) in registry.iter() {
        if path.starts_with("debug/") || path.contains("/debug/") {
            reasons
                .entry(*guid)
                .or_insert(InclusionReason::ProfileForced);
        }
    }
}

/// Resolves the scene file that seeds a profile closure.
pub fn seed_scene_path(
    project_root: &Path,
    manifest: &redox_editor_core::project::ProjectManifest,
    profile: &BuildProfile,
) -> Result<PathBuf, String> {
    let rel = profile
        .scene
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(manifest.start_scene.as_str());
    if rel.is_empty() {
        return Err("profile has no seed scene and project start_scene is empty".into());
    }
    let path = project_root.join(rel);
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("seed scene {} is missing", path.display()))
    }
}