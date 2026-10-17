//! World data for streaming: the one-file-per-actor (OFPA) on-disk layout, the `WorldPartition`
//! runtime streamer and deterministic procedural generation.

#![warn(missing_docs)]

use std::any::TypeId;
use std::collections::HashMap;
use std::fmt;
use std::io;

use redox_ecs::component::ComponentInfo;
use serde::{Deserialize, Serialize};

pub mod ofpa;
pub mod partition;
pub mod pcg;

pub use ofpa::{
    ActorRecord, CELLS_DIR, WORLD_FILE, WorldManifest, actor_path, cell_dir, cell_mtime,
    load_cell_actors, load_manifest, parse_cell_dir, save_actor, save_cell_actors, save_manifest,
    write_atomic, write_world,
};

pub use partition::{
    StreamedActor, StreamingBudget, StreamingConfig, StreamingReport, WorldPartition,
    tick_streaming,
};
pub use pcg::{GeneratedWorld, PcgKind, PcgParams, generate, generate_to};
pub use redox_reflect::{EntitySnapshot, ReflectValue, TypeRegistry, WorldSnapshot};

/// Result type of the scene crate.
pub type SceneResult<T> = Result<T, SceneError>;

/// Errors from reading or writing world files.
#[derive(Debug)]
pub enum SceneError {
    /// File-system error.
    Io(io::Error),
    /// RON serialization error.
    Ron(ron::Error),
    /// RON parse error with a source position.
    RonSpanned(ron::error::SpannedError),
    /// The world has no `world.ron` manifest (path attached).
    MissingManifest(String),
    /// A cell directory or file is malformed (message attached).
    InvalidCell(String),
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Ron(e) => write!(f, "ron: {e}"),
            Self::RonSpanned(e) => write!(f, "ron: {e}"),
            Self::MissingManifest(s) => write!(f, "missing manifest: {s}"),
            Self::InvalidCell(s) => write!(f, "invalid cell: {s}"),
        }
    }
}

impl std::error::Error for SceneError {}

impl From<io::Error> for SceneError {
    fn from(v: io::Error) -> Self {
        Self::Io(v)
    }
}
impl From<ron::Error> for SceneError {
    fn from(v: ron::Error) -> Self {
        Self::Ron(v)
    }
}
impl From<ron::error::SpannedError> for SceneError {
    fn from(v: ron::error::SpannedError) -> Self {
        Self::RonSpanned(v)
    }
}

/// Maps component type IDs to ECS metadata for serialization and OFPA export.
#[derive(Debug, Default)]
pub struct ComponentRegistry {
    entries: HashMap<TypeId, ComponentRegistryEntry>,
}

/// Registered component type with its runtime metadata.
#[derive(Debug, Clone)]
pub struct ComponentRegistryEntry {
    /// Rust type name.
    pub type_name: &'static str,
    /// ECS metadata (size, alignment, drop glue).
    pub info: ComponentInfo,
}

impl ComponentRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }
    /// Registers component type `T`.
    pub fn register<T: redox_ecs::Component + 'static>(&mut self) {
        self.entries.insert(
            TypeId::of::<T>(),
            ComponentRegistryEntry {
                type_name: std::any::type_name::<T>(),
                info: ComponentInfo::of::<T>(),
            },
        );
    }
    /// Looks up an entry by type id.
    pub fn get(&self, type_id: TypeId) -> Option<&ComponentRegistryEntry> {
        self.entries.get(&type_id)
    }
    /// Iterates over all registered components.
    pub fn iter(&self) -> impl Iterator<Item = (&TypeId, &ComponentRegistryEntry)> {
        self.entries.iter()
    }
    /// Syncs known reflectable types from [`redox_reflect::TypeRegistry`] into the scene registry.
    pub fn sync_from_type_registry(&mut self, registry: &redox_reflect::TypeRegistry) {
        for name in registry.type_names() {
            match name {
                "Transform" => self.register::<redox_math::Transform>(),
                "StreamedActor" => self.register::<StreamedActor>(),
                _ => {}
            }
        }
    }
    /// Registers the components spawned by [`WorldPartition`].
    pub fn with_streaming_defaults() -> Self {
        let mut r = Self::new();
        r.register::<StreamedActor>();
        r.register::<redox_math::Transform>();
        r
    }
}

/// World-space partition cell identifier (XZ grid, Y unused).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PartitionCell {
    /// Column of the cell along X.
    pub x: i32,
    /// Row of the cell along Z.
    pub y: i32,
}

impl PartitionCell {
    /// Cell containing the world position `pos` for a grid of `cell_size` anchored at `origin`.
    pub fn from_world(cell_size: f32, origin: [f32; 3], pos: [f32; 3]) -> Self {
        let size = cell_size.max(1e-4);
        let x = ((pos[0] - origin[0]) / size).floor() as i32;
        let y = ((pos[2] - origin[2]) / size).floor() as i32;
        Self { x, y }
    }
    /// Cell containing `pos` for a grid anchored at the world origin.
    pub fn from_world_pos(pos: redox_math::Vec3, cell_size: f32) -> Self {
        Self::from_world(cell_size, [0.0; 3], pos.to_array())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partition_cell_roundtrip() {
        let c = PartitionCell { x: 1, y: -2 };
        assert_eq!(
            c,
            PartitionCell::from_world(16.0, [0.0; 3], [24.0, 0.0, -20.0])
        );
    }
}
