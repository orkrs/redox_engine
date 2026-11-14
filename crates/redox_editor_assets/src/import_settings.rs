//! Per-asset import settings and named presets (EDT-B4 step 1).

use std::fs;
use std::path::Path;

use redox_asset::{AssetMeta, AssetRegistry, RegistryError};

use crate::browser_model::AssetKind;

/// Schema version of [`PerAssetImportSettings`] stored in `.meta`.
pub const SETTINGS_SCHEMA: u32 = 1;

/// Named import presets shown in the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ImportPresetId {
    /// Default glTF import.
    ModelDefault,
    /// Lighter meshes for realtime previews.
    ModelRealtime,
    /// Albedo or mask textures.
    TextureColor,
    /// Normal or data maps.
    TextureNormal,
}

impl ImportPresetId {
    /// Stable id written to project preset files.
    pub fn name(self) -> &'static str {
        match self {
            Self::ModelDefault => "Model/Default",
            Self::ModelRealtime => "Model/Realtime",
            Self::TextureColor => "Texture/Color",
            Self::TextureNormal => "Texture/Normal",
        }
    }

    /// Parses a preset name from RON or the UI.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "Model/Default" => Some(Self::ModelDefault),
            "Model/Realtime" => Some(Self::ModelRealtime),
            "Texture/Color" => Some(Self::TextureColor),
            "Texture/Normal" => Some(Self::TextureNormal),
            _ => None,
        }
    }

    /// Every built-in preset id.
    pub fn all() -> &'static [Self] {
        &[
            Self::ModelDefault,
            Self::ModelRealtime,
            Self::TextureColor,
            Self::TextureNormal,
        ]
    }
}

/// How materials are imported from glTF.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MaterialMode {
    /// Engine default shading.
    #[default]
    Default,
    /// Metallic-roughness workflow emphasized.
    MetallicRoughness,
    /// Unlit/emissive friendly.
    Unlit,
}

/// Import knobs stored per asset in the sidecar `settings` field (RON).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PerAssetImportSettings {
    /// Serialisation version for forward-compatible defaults.
    pub version: u32,
    /// Preset selected in the UI.
    pub preset: ImportPresetId,
    /// Uniform scale applied on import (models).
    #[serde(default = "default_scale")]
    pub scale: f32,
    /// Whether mips are generated for textures.
    #[serde(default = "default_true")]
    pub generate_mipmaps: bool,
    /// Material interpretation for meshes.
    #[serde(default)]
    pub material_mode: MaterialMode,
    /// Fingerprint recorded at the last successful import (0 = never imported with settings).
    #[serde(default)]
    pub imported_fingerprint: u64,
}

fn default_scale() -> f32 {
    1.0
}

fn default_true() -> bool {
    true
}

impl Default for PerAssetImportSettings {
    fn default() -> Self {
        settings_for_preset(ImportPresetId::ModelDefault)
    }
}

/// Settings implied by a preset id.
pub fn settings_for_preset(preset: ImportPresetId) -> PerAssetImportSettings {
    let (scale, mips, material_mode) = match preset {
        ImportPresetId::ModelDefault => (1.0, true, MaterialMode::Default),
        ImportPresetId::ModelRealtime => (1.0, false, MaterialMode::MetallicRoughness),
        ImportPresetId::TextureColor => (1.0, true, MaterialMode::Default),
        ImportPresetId::TextureNormal => (1.0, true, MaterialMode::Default),
    };
    PerAssetImportSettings {
        version: SETTINGS_SCHEMA,
        preset,
        scale,
        generate_mipmaps: mips,
        material_mode,
        imported_fingerprint: 0,
    }
}

/// Default settings for a file kind when the sidecar has no blob yet.
pub fn default_for_kind(kind: AssetKind) -> PerAssetImportSettings {
    match kind {
        AssetKind::Mesh => settings_for_preset(ImportPresetId::ModelDefault),
        AssetKind::Texture => settings_for_preset(ImportPresetId::TextureColor),
        _ => PerAssetImportSettings::default(),
    }
}

/// Hash of the knobs that should trigger a re-import when they change.
pub fn settings_fingerprint(settings: &PerAssetImportSettings) -> u64 {
    redox_asset::guid::content_hash(settings_blob(settings).as_bytes())
}

/// Serialises settings for the sidecar (pretty RON).
pub fn settings_blob(settings: &PerAssetImportSettings) -> String {
    ron::ser::to_string_pretty(settings, ron::ser::PrettyConfig::new()).unwrap_or_default()
}

/// Parses the sidecar blob; missing or invalid data becomes kind-appropriate defaults.
pub fn parse_settings_blob(blob: &str, kind: AssetKind) -> PerAssetImportSettings {
    if blob.trim().is_empty() {
        return default_for_kind(kind);
    }
    match ron::from_str::<PerAssetImportSettings>(blob) {
        Ok(mut s) => {
            if s.version == 0 {
                s.version = SETTINGS_SCHEMA;
            }
            s
        }
        Err(_) => default_for_kind(kind),
    }
}

/// Reads per-asset settings from the registry sidecar.
pub fn read_per_asset_settings(
    registry: &AssetRegistry,
    path: &str,
) -> Result<PerAssetImportSettings, RegistryError> {
    let meta = registry.read_meta(path)?;
    let kind = AssetKind::from_path(registry.root().join(path).as_path());
    Ok(parse_settings_blob(&meta.settings, kind))
}

/// Writes settings to the sidecar; returns whether the asset should be marked outdated.
pub fn write_per_asset_settings(
    registry: &AssetRegistry,
    path: &str,
    settings: &PerAssetImportSettings,
) -> Result<bool, RegistryError> {
    let mut meta = registry.read_meta(path)?;
    let old = parse_settings_blob(
        &meta.settings,
        AssetKind::from_path(registry.root().join(path).as_path()),
    );
    let blob = settings_blob(settings);
    meta.settings = blob;
    registry.write_meta(path, &meta)?;
    let needs_reimport = settings_fingerprint(settings) != settings_fingerprint(&old)
        && old.imported_fingerprint != 0;
    Ok(needs_reimport)
}

/// After a successful import, records the fingerprint in the sidecar.
pub fn record_imported_settings(
    registry: &AssetRegistry,
    path: &str,
    settings: &PerAssetImportSettings,
) -> Result<(), RegistryError> {
    let mut next = settings.clone();
    next.imported_fingerprint = settings_fingerprint(settings);
    let mut meta = registry.read_meta(path)?;
    meta.settings = settings_blob(&next);
    registry.write_meta(path, &meta)
}

/// Optional project override file: `<project>/.redox/import_presets.ron`.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ImportPresetFile {
    /// Overrides keyed by [`ImportPresetId::name`].
    #[serde(default)]
    pub presets: Vec<PresetEntry>,
}

/// One named preset in the project file.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PresetEntry {
    /// Name such as `Model/Default`.
    pub name: String,
    /// Overrides merged onto the built-in defaults.
    #[serde(default)]
    pub settings: PerAssetImportSettings,
}

/// Built-in and project import presets.
#[derive(Clone, Debug)]
pub struct ImportPresetLibrary {
    entries: Vec<(ImportPresetId, PerAssetImportSettings)>,
}

impl ImportPresetLibrary {
    /// Built-in defaults only.
    pub fn builtins() -> Self {
        let entries = ImportPresetId::all()
            .iter()
            .map(|&id| (id, settings_for_preset(id)))
            .collect();
        Self { entries }
    }

    /// Loads `<project>/.redox/import_presets.ron` when present.
    pub fn load_project(project_root: &Path) -> Self {
        let mut lib = Self::builtins();
        let path = project_root.join(".redox/import_presets.ron");
        let Ok(text) = fs::read_to_string(&path) else {
            return lib;
        };
        let Ok(file) = ron::from_str::<ImportPresetFile>(&text) else {
            return lib;
        };
        for entry in file.presets {
            let Some(id) = ImportPresetId::parse(&entry.name) else {
                continue;
            };
            let mut merged = settings_for_preset(id);
            merged.scale = entry.settings.scale;
            merged.generate_mipmaps = entry.settings.generate_mipmaps;
            merged.material_mode = entry.settings.material_mode;
            if let Some((_, slot)) = lib.entries.iter_mut().find(|(pid, _)| *pid == id) {
                *slot = merged;
            }
        }
        lib
    }

    /// Effective settings for a preset id.
    pub fn get(&self, id: ImportPresetId) -> PerAssetImportSettings {
        self.entries
            .iter()
            .find(|(pid, _)| *pid == id)
            .map(|(_, s)| s.clone())
            .unwrap_or_else(|| settings_for_preset(id))
    }
}

/// Ensures a brand-new sidecar carries default import settings for its kind.
pub fn ensure_default_meta_settings(meta: &mut AssetMeta, kind: AssetKind) {
    if meta.settings.trim().is_empty() {
        meta.settings = settings_blob(&default_for_kind(kind));
    }
}
