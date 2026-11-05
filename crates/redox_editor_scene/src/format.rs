//! On-disk layout of `*.scene.ron` files: the format version, the entity list and migrations.

use redox_reflect::ReflectValue;
use redox_scene::EntityGuid;
use serde::{Deserialize, Serialize};

/// Format version written by this build. Files with a higher version are refused.
pub const SCENE_FORMAT_VERSION: u32 = 1;

/// A whole `*.scene.ron` file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneFile {
    /// Format version the file was written with (`0` when the field is missing).
    #[serde(default)]
    pub format_version: u32,
    /// Display name of the scene.
    #[serde(default)]
    pub name: String,
    /// Entities, in the order they were written.
    #[serde(default)]
    pub entities: Vec<SceneEntity>,
}

/// One entity of a scene file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneEntity {
    /// Stable identity. `None` only in hand-written or damaged files; the loader assigns one.
    #[serde(default)]
    pub guid: Option<EntityGuid>,
    /// Components of known types, one reflected struct each.
    #[serde(default)]
    pub components: Vec<ReflectValue>,
    /// Components of unknown types, kept as read so that saving does not lose them.
    #[serde(default)]
    pub unknown_components: Vec<ReflectValue>,
}

/// Upgrades `file` from format version `from` to [`SCENE_FORMAT_VERSION`], one step at a time.
pub fn migrate(file: &mut SceneFile, from: u32) {
    for version in from..SCENE_FORMAT_VERSION {
        migrate_step(file, version);
    }
    file.format_version = SCENE_FORMAT_VERSION;
}

/// Applies the single step from `from` to `from + 1`.
fn migrate_step(_file: &mut SceneFile, from: u32) {
    match from {
        // v0 -> v1: the layout is unchanged; v0 files only lacked the `format_version` field.
        0 => {}
        other => unreachable!("no migration step from format v{other}"),
    }
}
