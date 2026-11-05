//! Asset references inside scene files: finding them, resolving them by GUID, rewriting them,
//! and the index of which scenes refer to which assets.
//!
//! An asset reference is a reflected struct whose type name ends with `Handle` and which has a
//! `guid` or `path` field (see `ReflectValue::asset_ref`). The GUID is the source of truth; the
//! path is a hint that is rewritten after moves.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use redox_asset::guid::AssetStatus;
use redox_asset::{AssetGuid, AssetRegistry, DependentRef};
use redox_reflect::ReflectValue;
use serde::{Deserialize, Serialize};

/// Suffix of scene files.
pub const SCENE_SUFFIX: &str = ".scene.ron";

/// Whether a path (relative to the assets root) names a scene file.
pub fn is_scene_path(path: &str) -> bool {
    path.ends_with(SCENE_SUFFIX)
}

/// Failure to read, parse or write a scene file.
#[derive(Debug, thiserror::Error)]
pub enum ReferenceError {
    /// The file could not be read or written.
    #[error("{path}: {source}")]
    Io {
        /// File involved.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// The file is not a readable scene file.
    #[error("{path}: cannot parse scene: {message}")]
    Parse {
        /// File involved.
        path: PathBuf,
        /// Parser message.
        message: String,
    },
    /// The rewritten scene could not be serialized.
    #[error("{path}: cannot write scene: {message}")]
    Write {
        /// File involved.
        path: PathBuf,
        /// Serializer message.
        message: String,
    },
}

/// Minimal model of a `*.scene.ron` file: only what the asset service reads and rewrites.
///
/// Top-level fields that are not modelled here are not preserved by [`edit_scene_refs`]; the
/// scene document of EDT-A2 replaces this model once it is in place.
#[derive(Debug, Serialize, Deserialize)]
struct SceneFileModel {
    #[serde(default)]
    format_version: u32,
    #[serde(default)]
    name: String,
    #[serde(default)]
    entities: Vec<SceneEntityModel>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SceneEntityModel {
    #[serde(default)]
    guid: Option<String>,
    #[serde(default)]
    components: Vec<ReflectValue>,
    #[serde(default)]
    unknown_components: Vec<ReflectValue>,
}

/// An asset reference found in a scene file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SceneRef {
    /// GUID of the entity that holds the reference.
    pub entity: Option<String>,
    /// Registered name of the component that holds the reference.
    pub component: String,
    /// Referenced GUID text; `None` for the old path-only format.
    pub guid: Option<String>,
    /// Path hint; `None` when unset.
    pub path: Option<String>,
}

/// A reference whose asset cannot be found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingRef {
    /// Scene file that holds the reference.
    pub scene: String,
    /// GUID of the entity that holds the reference.
    pub entity: Option<String>,
    /// Registered name of the component that holds the reference.
    pub component: String,
    /// Referenced GUID text, if the reference had one.
    pub guid: Option<String>,
    /// Path hint, if the reference had one.
    pub path: Option<String>,
}

/// Result of resolving an asset reference against the registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefResolution {
    /// The GUID is registered; `path` is its current path.
    Resolved {
        /// Identity of the asset.
        guid: AssetGuid,
        /// Current path relative to the root.
        path: String,
    },
    /// The reference had no usable GUID, but its path names a registered asset; the GUID is
    /// restored from the registry.
    Restored {
        /// Identity restored from the path.
        guid: AssetGuid,
        /// Path relative to the root.
        path: String,
    },
    /// Neither the GUID nor the path points to an existing asset.
    Missing {
        /// Referenced GUID text, if any.
        guid: Option<String>,
        /// Path hint, if any.
        path: Option<String>,
    },
    /// The reference is unset (no GUID and no path).
    Unset,
}

/// Resolves a reference by GUID first, then by its path hint.
///
/// A GUID that is registered but missing on disk does not resolve; a path hint that names a
/// missing asset does not resolve either.
pub fn resolve_ref(
    registry: &AssetRegistry,
    guid_text: Option<&str>,
    path_hint: Option<&str>,
) -> RefResolution {
    if guid_text.is_none() && path_hint.is_none() {
        return RefResolution::Unset;
    }
    if let Some(guid) = guid_text.and_then(|t| t.parse::<AssetGuid>().ok())
        && let Some(path) = registry.path_of(guid)
        && registry.status(path) != Some(&AssetStatus::Missing)
    {
        return RefResolution::Resolved {
            guid,
            path: path.to_owned(),
        };
    }
    if let Some(hint) = path_hint
        && let Some(guid) = registry.guid_of(hint)
        && registry.status(hint) != Some(&AssetStatus::Missing)
    {
        return RefResolution::Restored {
            guid,
            path: hint.to_owned(),
        };
    }
    RefResolution::Missing {
        guid: guid_text.map(str::to_owned),
        path: path_hint.map(str::to_owned),
    }
}

/// GUID of the asset a reference points to, even when that asset is missing on disk.
/// Used for the reverse index, so that "show references" also works for missing assets.
fn known_guid(registry: &AssetRegistry, sref: &SceneRef) -> Option<AssetGuid> {
    if let Some(guid) = sref.guid.as_deref().and_then(|t| t.parse::<AssetGuid>().ok())
        && registry.path_of(guid).is_some()
    {
        return Some(guid);
    }
    sref.path
        .as_deref()
        .and_then(|hint| registry.guid_of(hint))
}

/// What to do with one asset reference while a scene file is rewritten.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefEdit {
    /// Keep the reference unchanged.
    Keep,
    /// Set the path hint; the GUID stays.
    SetPath(String),
    /// Point the reference at another asset.
    Reassign {
        /// New GUID text.
        guid: String,
        /// New path hint, if known.
        path: Option<String>,
    },
    /// Clear the reference (GUID and path become null).
    Clear,
}

fn component_name(value: &ReflectValue) -> String {
    match value {
        ReflectValue::Struct { type_name, .. } => type_name.clone(),
        _ => String::new(),
    }
}

fn is_asset_ref(value: &ReflectValue) -> bool {
    matches!(value, ReflectValue::Struct { type_name, .. } if type_name.ends_with("Handle"))
        && (value.field("guid").is_some() || value.field("path").is_some())
}

fn snapshot(value: &ReflectValue, entity: Option<&str>, component: &str) -> SceneRef {
    SceneRef {
        entity: entity.map(str::to_owned),
        component: component.to_owned(),
        guid: value
            .field("guid")
            .and_then(ReflectValue::as_string)
            .map(str::to_owned),
        path: value
            .field("path")
            .and_then(ReflectValue::as_string)
            .map(str::to_owned),
    }
}

fn set_field(fields: &mut [(String, ReflectValue)], name: &str, value: ReflectValue) {
    if let Some(slot) = fields.iter_mut().find(|(n, _)| n == name) {
        slot.1 = value;
    }
}

fn optional_string(text: Option<String>) -> ReflectValue {
    text.map_or(ReflectValue::Null, ReflectValue::String)
}

/// Applies `edit` to one reference value; returns whether the value changed.
fn apply_edit(value: &mut ReflectValue, edit: &RefEdit) -> bool {
    let before = value.clone();
    let ReflectValue::Struct { fields, .. } = value else {
        return false;
    };
    match edit {
        RefEdit::Keep => {}
        RefEdit::SetPath(path) => set_field(fields, "path", ReflectValue::String(path.clone())),
        RefEdit::Reassign { guid, path } => {
            set_field(fields, "guid", ReflectValue::String(guid.clone()));
            set_field(fields, "path", optional_string(path.clone()));
        }
        RefEdit::Clear => {
            set_field(fields, "guid", ReflectValue::Null);
            set_field(fields, "path", ReflectValue::Null);
        }
    }
    before != *value
}

/// Walks a reflected value and calls `edit` for each asset reference; counts changed ones.
fn visit_refs<F>(
    value: &mut ReflectValue,
    entity: Option<&str>,
    component: &str,
    edit: &mut F,
    changed: &mut usize,
) where
    F: FnMut(&SceneRef) -> RefEdit,
{
    if is_asset_ref(value) {
        let decision = edit(&snapshot(value, entity, component));
        if apply_edit(value, &decision) {
            *changed += 1;
        }
        return;
    }
    match value {
        ReflectValue::Struct { fields, .. } => {
            for (_, child) in fields.iter_mut() {
                visit_refs(child, entity, component, edit, changed);
            }
        }
        ReflectValue::List(items) => {
            for child in items.iter_mut() {
                visit_refs(child, entity, component, edit, changed);
            }
        }
        _ => {}
    }
}

fn read_model(path: &Path) -> Result<SceneFileModel, ReferenceError> {
    let text = fs::read_to_string(path).map_err(|source| ReferenceError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    ron::from_str(&text).map_err(|e| ReferenceError::Parse {
        path: path.to_path_buf(),
        message: e.to_string(),
    })
}

/// Lists the asset references of one scene file (components only).
fn scan_scene(path: &Path) -> Result<Vec<SceneRef>, ReferenceError> {
    let model = read_model(path)?;
    let mut refs = Vec::new();
    for entity in &model.entities {
        for component in &entity.components {
            let name = component_name(component);
            let mut owned = component.clone();
            let mut collect = |sref: &SceneRef| {
                refs.push(sref.clone());
                RefEdit::Keep
            };
            visit_refs(&mut owned, entity.guid.as_deref(), &name, &mut collect, &mut 0);
        }
    }
    Ok(refs)
}

/// Rewrites the asset references of one scene file (path relative to `root`).
///
/// `edit` is called for every asset reference in the components. The file is rewritten only when
/// something changed, atomically, keeping a `.bak` copy of the previous content (see
/// `redox_scene::ofpa::write_atomic`). Returns the number of changed references.
pub fn edit_scene_refs<F>(root: &Path, scene: &str, mut edit: F) -> Result<usize, ReferenceError>
where
    F: FnMut(&SceneRef) -> RefEdit,
{
    let path = root.join(scene);
    let mut model = read_model(&path)?;
    let mut changed = 0;
    for entity in &mut model.entities {
        let entity_guid = entity.guid.clone();
        for component in &mut entity.components {
            let name = component_name(component);
            visit_refs(
                component,
                entity_guid.as_deref(),
                &name,
                &mut edit,
                &mut changed,
            );
        }
    }
    if changed > 0 {
        let text = ron::ser::to_string_pretty(&model, ron::ser::PrettyConfig::new()).map_err(
            |e| ReferenceError::Write {
                path: path.clone(),
                message: e.to_string(),
            },
        )?;
        redox_scene::ofpa::write_atomic(&path, text.as_bytes()).map_err(|source| {
            ReferenceError::Io {
                path: path.clone(),
                source,
            }
        })?;
    }
    Ok(changed)
}

/// Index of the scenes' asset references.
///
/// The dependents (reverse references) are stored in the registry; this index keeps the parsed
/// references per scene, so that missing references can be listed without re-reading files.
#[derive(Debug, Default)]
pub struct ReferenceIndex {
    scenes: BTreeMap<String, Vec<SceneRef>>,
}

impl ReferenceIndex {
    /// Re-indexes every scene file of the registry. Returns the scenes that could not be read.
    pub fn rebuild(&mut self, registry: &mut AssetRegistry) -> Vec<ReferenceError> {
        self.scenes.clear();
        let scenes: Vec<String> = registry
            .iter()
            .filter(|(p, _)| is_scene_path(p))
            .map(|(p, _)| p.to_owned())
            .collect();
        let mut errors = Vec::new();
        for scene in scenes {
            if let Err(err) = self.index_scene(registry, &scene) {
                errors.push(err);
            }
        }
        errors
    }

    /// Re-indexes one scene file after it changed.
    pub fn update_scene(
        &mut self,
        registry: &mut AssetRegistry,
        scene: &str,
    ) -> Result<(), ReferenceError> {
        self.index_scene(registry, scene)
    }

    /// Forgets a scene file that was deleted or moved away.
    pub fn remove_scene(&mut self, registry: &mut AssetRegistry, scene: &str) {
        self.scenes.remove(scene);
        registry.remove_scene_references(scene);
    }

    fn index_scene(
        &mut self,
        registry: &mut AssetRegistry,
        scene: &str,
    ) -> Result<(), ReferenceError> {
        let path = registry.root().join(scene);
        if !path.exists() || registry.status(scene) == Some(&AssetStatus::Missing) {
            self.remove_scene(registry, scene);
            return Ok(());
        }
        let refs = scan_scene(&path)?;
        let dependents: Vec<DependentRef> = refs
            .iter()
            .filter_map(|sref| {
                known_guid(registry, sref).map(|guid| DependentRef {
                    guid,
                    scene: scene.to_owned(),
                    entity: sref.entity.clone(),
                    component: sref.component.clone(),
                })
            })
            .collect();
        registry.set_scene_references(scene, dependents);
        self.scenes.insert(scene.to_owned(), refs);
        Ok(())
    }

    /// References whose asset cannot be resolved right now, in scene path order.
    pub fn missing_refs(&self, registry: &AssetRegistry) -> Vec<MissingRef> {
        let mut out = Vec::new();
        for (scene, refs) in &self.scenes {
            for sref in refs {
                if let RefResolution::Missing { guid, path } =
                    resolve_ref(registry, sref.guid.as_deref(), sref.path.as_deref())
                {
                    out.push(MissingRef {
                        scene: scene.clone(),
                        entity: sref.entity.clone(),
                        component: sref.component.clone(),
                        guid,
                        path,
                    });
                }
            }
        }
        out
    }
}
