use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LightmapEntry {
    pub id: String,
    pub meters_per_texel: f32,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LightmapsManifest {
    pub entries: Vec<LightmapEntry>,
}

pub fn write_lightmaps_manifest(
    out_dir: &Path,
    manifest: &LightmapsManifest,
) -> Result<(), String> {
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let text = ron::ser::to_string_pretty(manifest, ron::ser::PrettyConfig::new())
        .map_err(|e| e.to_string())?;
    std::fs::write(out_dir.join("lightmaps.ron"), text).map_err(|e| e.to_string())
}

pub fn read_lightmaps_manifest(dir: &Path) -> Result<LightmapsManifest, String> {
    let text = std::fs::read_to_string(dir.join("lightmaps.ron")).map_err(|e| e.to_string())?;
    ron::from_str(&text).map_err(|e| e.to_string())
}
