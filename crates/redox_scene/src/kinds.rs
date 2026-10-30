//! Actor kind table: maps the `kind` string of an [`ActorRecord`] to the mesh
//! that draws it. The game already interprets `kind`; this table adds the
//! geometry link that the HLOD bake needs, without changing the actor format.
//!
//! The table is stored as RON (`actor_kinds.ron`) next to `world.bin`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use redox_math::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::ofpa::ActorRecord;

/// Geometry and HLOD participation of one actor kind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActorKindEntry {
    /// Asset path of the mesh, as in the `.pak` file.
    pub mesh: String,
    /// Uniform scale applied on top of the actor's transform.
    #[serde(default = "default_scale")]
    pub scale: f32,
    /// Average colour used by the HLOD proxy (linear RGB).
    pub color: [f32; 3],
    /// Whether actors of this kind take part in HLOD.
    #[serde(default = "default_hlod")]
    pub hlod: bool,
}

fn default_scale() -> f32 {
    1.0
}

fn default_hlod() -> bool {
    true
}

/// One actor placed in the world: the mesh to draw and its model matrix.
#[derive(Clone, Debug, PartialEq)]
pub struct HlodInstance {
    /// Asset path of the mesh (from [`ActorKindEntry::mesh`]).
    pub mesh: String,
    /// Average colour of the kind, used for the proxy.
    pub color: [f32; 3],
    /// Local-to-world transform built from position, yaw and scale.
    pub transform: Mat4,
}

/// Errors of reading an [`ActorKindTable`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KindTableError {
    /// The text is not a valid RON kind table (message attached).
    Ron(String),
}

impl fmt::Display for KindTableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ron(message) => write!(f, "invalid actor kind table: {message}"),
        }
    }
}

impl std::error::Error for KindTableError {}

/// All actor kinds of a world, keyed by kind name.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ActorKindTable {
    /// Entries keyed by [`ActorRecord::kind`]. Ordered for deterministic output.
    pub kinds: BTreeMap<String, ActorKindEntry>,
}

impl ActorKindTable {
    /// Parses a table from RON text.
    ///
    /// # Errors
    /// [`KindTableError::Ron`] when the text is not a valid table.
    pub fn from_ron(text: &str) -> Result<Self, KindTableError> {
        ron::from_str(text).map_err(|e| KindTableError::Ron(e.to_string()))
    }

    /// Serialises the table as pretty RON.
    pub fn to_ron(&self) -> Result<String, KindTableError> {
        let pretty = ron::ser::PrettyConfig::new();
        ron::ser::to_string_pretty(self, pretty).map_err(|e| KindTableError::Ron(e.to_string()))
    }

    /// The entry of a kind, if the table has one.
    pub fn get(&self, kind: &str) -> Option<&ActorKindEntry> {
        self.kinds.get(kind)
    }

    /// Whether actors of `kind` take part in HLOD (false for unknown kinds).
    pub fn is_hlod(&self, kind: &str) -> bool {
        self.get(kind).is_some_and(|entry| entry.hlod)
    }

    /// The HLOD instance of one actor, or `None` when its kind is unknown or
    /// excluded from HLOD.
    pub fn to_instance(&self, actor: &ActorRecord) -> Option<HlodInstance> {
        let entry = self.get(&actor.kind).filter(|entry| entry.hlod)?;
        let transform = Mat4::from_scale_rotation_translation(
            Vec3::splat(entry.scale),
            Quat::from_rotation_y(actor.yaw),
            Vec3::from_array(actor.position),
        );
        Some(HlodInstance {
            mesh: entry.mesh.clone(),
            color: entry.color,
            transform,
        })
    }

    /// Instances of all actors of a cell, plus the kinds that had no entry
    /// (so the caller can warn once per kind). Actors of kinds with
    /// `hlod: false` are silently left out.
    pub fn to_instances(&self, actors: &[ActorRecord]) -> (Vec<HlodInstance>, BTreeSet<String>) {
        let mut instances = Vec::new();
        let mut unknown = BTreeSet::new();
        for actor in actors {
            match self.get(&actor.kind) {
                Some(_) => instances.extend(self.to_instance(actor)),
                None => {
                    unknown.insert(actor.kind.clone());
                }
            }
        }
        (instances, unknown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = r#"(
        kinds: {
            "crate": (mesh: "meshes/crate.glb", scale: 2.0, color: (0.5, 0.25, 0.0), hlod: true),
            "tree": (mesh: "meshes/tree.glb", color: (0.1, 0.6, 0.1), hlod: false),
        },
    )"#;

    fn actor(kind: &str, position: [f32; 3], yaw: f32) -> ActorRecord {
        ActorRecord {
            id: format!("{kind}_1"),
            kind: kind.to_string(),
            position,
            yaw,
            tags: Vec::new(),
        }
    }

    #[test]
    fn a_table_parses_from_ron() {
        let table = ActorKindTable::from_ron(TABLE).unwrap();
        let crate_entry = table.get("crate").unwrap();
        assert_eq!(crate_entry.mesh, "meshes/crate.glb");
        assert_eq!(crate_entry.scale, 2.0);
        assert!(table.get("tree").is_some());
        assert!(table.get("rock").is_none());
    }

    #[test]
    fn a_defaulted_entry_uses_unit_scale_and_hlod_on() {
        let table = ActorKindTable::from_ron(
            r#"(kinds: { "rock": (mesh: "meshes/rock.glb", color: (0.4, 0.4, 0.4)) })"#,
        )
        .unwrap();
        let rock = table.get("rock").unwrap();
        assert_eq!(rock.scale, 1.0);
        assert!(rock.hlod);
    }

    #[test]
    fn an_unknown_kind_is_skipped_with_a_warning() {
        let table = ActorKindTable::from_ron(TABLE).unwrap();
        let actors = [actor("crate", [0.0; 3], 0.0), actor("rock", [1.0; 3], 0.0)];
        let (instances, unknown) = table.to_instances(&actors);
        assert_eq!(instances.len(), 1);
        assert_eq!(unknown.into_iter().collect::<Vec<_>>(), vec!["rock"]);
    }

    #[test]
    fn to_instance_applies_position_yaw_and_scale() {
        let table = ActorKindTable::from_ron(TABLE).unwrap();
        let instance = table
            .to_instance(&actor("crate", [3.0, 1.0, -4.0], std::f32::consts::FRAC_PI_2))
            .unwrap();
        let expected = Mat4::from_scale_rotation_translation(
            Vec3::splat(2.0),
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            Vec3::new(3.0, 1.0, -4.0),
        );
        assert_eq!(instance.mesh, "meshes/crate.glb");
        assert!(instance.transform.abs_diff_eq(expected, 1e-5));
        // A point one unit along local +X lands 2 units along the rotated +X (-Z).
        let p = instance.transform.transform_point3(Vec3::X);
        assert!(p.abs_diff_eq(Vec3::new(3.0, 1.0, -6.0), 1e-4));
    }

    #[test]
    fn a_kind_with_hlod_false_is_excluded() {
        let table = ActorKindTable::from_ron(TABLE).unwrap();
        assert!(!table.is_hlod("tree"));
        assert!(table.is_hlod("crate"));
        assert!(table.to_instance(&actor("tree", [0.0; 3], 0.0)).is_none());
        let (instances, unknown) = table.to_instances(&[actor("tree", [0.0; 3], 0.0)]);
        assert!(instances.is_empty());
        assert!(unknown.is_empty());
    }

    #[test]
    fn the_table_roundtrips_through_ron() {
        let table = ActorKindTable::from_ron(TABLE).unwrap();
        let text = table.to_ron().unwrap();
        assert_eq!(ActorKindTable::from_ron(&text).unwrap(), table);
    }

    #[test]
    fn a_malformed_table_is_an_error() {
        assert!(matches!(
            ActorKindTable::from_ron("(kinds: {"),
            Err(KindTableError::Ron(_))
        ));
    }
}
