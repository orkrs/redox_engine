use std::any::TypeId;
use std::collections::HashMap;
use std::fmt;
use std::io;

use redox_ecs::component::ComponentInfo;
use serde::{Deserialize, Serialize};

pub mod ofpa;
pub mod partition;
pub mod pcg;

pub use ofpa::{ActorRecord, WorldManifest, cell_dir, actor_path, parse_cell_dir, load_manifest, save_manifest, load_cell_actors, save_cell_actors, save_actor, cell_mtime, write_world, write_atomic, CELLS_DIR, WORLD_FILE};

pub use partition::{WorldPartition, StreamedActor, StreamingBudget, StreamingConfig, StreamingReport, tick_streaming};
pub use pcg::{generate, generate_to, GeneratedWorld, PcgKind, PcgParams};
pub use redox_reflect::{EntitySnapshot, ReflectValue, TypeRegistry, WorldSnapshot};

pub type SceneResult<T> = Result<T, SceneError>;

#[derive(Debug)]
pub enum SceneError {
    Io(io::Error),
    Ron(ron::Error),
    RonSpanned(ron::error::SpannedError),
    MissingManifest(String),
    InvalidCell(String),
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self { Self::Io(e) => write!(f, "io: {e}"), Self::Ron(e) => write!(f, "ron: {e}"), Self::RonSpanned(e) => write!(f, "ron: {e}"), Self::MissingManifest(s) => write!(f, "missing manifest: {s}"), Self::InvalidCell(s) => write!(f, "invalid cell: {s}") }
    }
}

impl std::error::Error for SceneError {}

impl From<io::Error> for SceneError { fn from(v: io::Error) -> Self { Self::Io(v) } }
impl From<ron::Error> for SceneError { fn from(v: ron::Error) -> Self { Self::Ron(v) } }
impl From<ron::error::SpannedError> for SceneError { fn from(v: ron::error::SpannedError) -> Self { Self::RonSpanned(v) } }

/// Maps component type IDs to ECS metadata for serialization and OFPA export.
#[derive(Debug, Default)]
pub struct ComponentRegistry {
    entries: HashMap<TypeId, ComponentRegistryEntry>,
}

#[derive(Debug, Clone)]
pub struct ComponentRegistryEntry {
    pub type_name: &'static str,
    pub info: ComponentInfo,
}

impl ComponentRegistry {
    pub fn new() -> Self { Self::default() }
    pub fn register<T: redox_ecs::Component + 'static>(&mut self) {
        self.entries.insert(TypeId::of::<T>(), ComponentRegistryEntry { type_name: std::any::type_name::<T>(), info: ComponentInfo::of::<T>() });
    }
    pub fn get(&self, type_id: TypeId) -> Option<&ComponentRegistryEntry> { self.entries.get(&type_id) }
    pub fn iter(&self) -> impl Iterator<Item = (&TypeId, &ComponentRegistryEntry)> { self.entries.iter() }
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
    pub x: i32,
    pub y: i32,
}

impl PartitionCell {
    pub fn from_world(cell_size: f32, origin: [f32; 3], pos: [f32; 3]) -> Self {
        let size = cell_size.max(1e-4);
        let x = ((pos[0] - origin[0]) / size).floor() as i32;
        let y = ((pos[2] - origin[2]) / size).floor() as i32;
        Self { x, y }
    }
    pub fn from_world_pos(pos: redox_math::Vec3, cell_size: f32) -> Self { Self::from_world(cell_size, [0.0;3], pos.to_array()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn partition_cell_roundtrip() { let c = PartitionCell { x: 1, y: -2 }; assert_eq!(c, PartitionCell::from_world(16.0, [0.0;3], [24.0, 0.0, -20.0])); }
}
