//! Type reflection and RON snapshots for registered ECS components.

use std::any::TypeId;
use std::collections::HashMap;

use redox_ecs::{Entity, World};
use serde::{Deserialize, Serialize};

mod impls;

pub use impls::register_engine_defaults;

/// Serializable, type-erased value used by the inspector and scene save.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ReflectValue {
    Null,
    Bool(bool),
    I64(i64),
    F64(f64),
    String(String),
    List(Vec<ReflectValue>),
    Struct {
        type_name: String,
        fields: Vec<(String, ReflectValue)>,
    },
}

impl ReflectValue {
    pub fn f32(v: f32) -> Self {
        Self::F64(v as f64)
    }

    pub fn vec3(x: f32, y: f32, z: f32) -> Self {
        Self::List(vec![Self::f32(x), Self::f32(y), Self::f32(z)])
    }

    pub fn quat(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self::List(vec![Self::f32(x), Self::f32(y), Self::f32(z), Self::f32(w)])
    }

    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Self::F64(v) => Some(*v as f32),
            Self::I64(v) => Some(*v as f32),
            _ => None,
        }
    }

    pub fn as_vec3(&self) -> Option<[f32; 3]> {
        match self {
            Self::List(items) if items.len() == 3 => {
                Some([items[0].as_f32()?, items[1].as_f32()?, items[2].as_f32()?])
            }
            _ => None,
        }
    }

    pub fn as_quat(&self) -> Option<[f32; 4]> {
        match self {
            Self::List(items) if items.len() == 4 => Some([
                items[0].as_f32()?,
                items[1].as_f32()?,
                items[2].as_f32()?,
                items[3].as_f32()?,
            ]),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        if let Self::Bool(b) = self { Some(*b) } else { None }
    }

    pub fn as_i64(&self) -> Option<i64> {
        if let Self::I64(v) = self { Some(*v) } else { None }
    }

    pub fn as_u32(&self) -> Option<u32> {
        match self {
            Self::I64(v) if *v >= 0 => Some(*v as u32),
            Self::F64(v) if *v >= 0.0 => Some(*v as u32),
            _ => None,
        }
    }

    pub fn as_string(&self) -> Option<&str> {
        if let Self::String(s) = self { Some(s) } else { None }
    }

    pub fn handle(type_name: impl Into<String>, asset_path: Option<String>) -> Self {
        Self::Struct { type_name: type_name.into(), fields: vec![("path".into(), asset_path.map(Self::String).unwrap_or(Self::Null))] }
    }

    pub fn as_handle_path(&self) -> Option<Option<String>> {
        match self {
            Self::Struct { fields, .. } => fields.iter().find(|(k,_)| k=="path").map(|(_,v)| match v { Self::String(s) => Some(s.clone()), Self::Null => None, _ => None }),
            _ => None,
        }
    }

    pub fn field(&self, name: &str) -> Option<&ReflectValue> {
        match self {
            Self::Struct { fields, .. } => fields
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v),
            _ => None,
        }
    }
}

/// Describes a reflectable type for the auto-inspector
#[derive(Clone, Debug, Default)]
pub struct ReflectSchema {
    pub type_name: &'static str,
    pub fields: Vec<ReflectField>,
}

#[derive(Clone, Debug)]
pub struct ReflectField {
    pub name: &'static str,
    pub type_name: &'static str,
}


#[derive(Debug)]
pub enum ReflectError {
    Message(String),
}

impl std::fmt::Display for ReflectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for ReflectError {}

impl ReflectError {
    pub fn msg(m: impl Into<String>) -> Self {
        Self::Message(m.into())
    }
}

/// Component that can be inspected and written to RON via [`TypeRegistry`].
pub trait Reflect: Send + Sync + 'static {
    fn type_name() -> &'static str
    where
        Self: Sized;
    fn to_value(&self) -> ReflectValue;
    fn apply(&mut self, value: &ReflectValue) -> Result<(), ReflectError>;
    fn reflect_schema() -> ReflectSchema
    where
        Self: Sized,
    {
        ReflectSchema { type_name: Self::type_name(), fields: Vec::new() }
    }
}

type InsertFn = fn(&mut World, Entity, &ReflectValue) -> Result<(), ReflectError>;
type ReadFn = fn(&World, Entity) -> Option<ReflectValue>;
type HasFn = fn(&World, Entity) -> bool;

#[derive(Clone)]
#[allow(dead_code)]
struct Registration {
    type_name: &'static str,
    #[allow(dead_code)]
    type_id: TypeId,
    insert: InsertFn,
    read: ReadFn,
    has: HasFn,
    schema: ReflectSchema,
}

/// Maps registered component types to read/write/RON helpers.
#[derive(Clone, Default)]
pub struct TypeRegistry {
    by_name: HashMap<&'static str, Registration>,
    by_id: HashMap<TypeId, Registration>,
}

impl TypeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<T: Reflect + Default + redox_ecs::Component>(&mut self) {
        let reg = Registration {
            type_name: T::type_name(),
            type_id: TypeId::of::<T>(),
            insert: insert_typed::<T>,
            read: read_typed::<T>,
            has: has_typed::<T>,
            schema: T::reflect_schema(),
        };
        self.by_name.insert(T::type_name(), reg.clone());
        self.by_id.insert(TypeId::of::<T>(), reg);
    }

    pub fn schema(&self, type_name: &str) -> Option<&ReflectSchema> {
        self.by_name.get(type_name).map(|r| &r.schema)
    }

    pub fn type_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.by_name.keys().copied()
    }

    pub fn insert_component(
        &self,
        world: &mut World,
        entity: Entity,
        value: &ReflectValue,
    ) -> Result<(), ReflectError> {
        let type_name = match value {
            ReflectValue::Struct { type_name, .. } => type_name.as_str(),
            _ => return Err(ReflectError::msg("expected struct ReflectValue")),
        };
        let reg = self
            .by_name
            .get(type_name)
            .ok_or_else(|| ReflectError::msg(format!("unregistered type {type_name}")))?;
        (reg.insert)(world, entity, value)
    }

    pub fn read_component(&self, world: &World, entity: Entity, type_name: &str) -> Option<ReflectValue> {
        let reg = self.by_name.get(type_name)?;
        (reg.read)(world, entity)
    }

    /// Serializes every registered component present on `entity`.
    pub fn snapshot_entity(&self, world: &World, entity: Entity) -> Vec<ReflectValue> {
        let mut out = Vec::new();
        for reg in self.by_id.values() {
            if (reg.has)(world, entity) {
                if let Some(v) = (reg.read)(world, entity) {
                    out.push(v);
                }
            }
        }
        out.sort_by(|a, b| match (a, b) {
            (
                ReflectValue::Struct { type_name: l, .. },
                ReflectValue::Struct { type_name: r, .. },
            ) => l.cmp(r),
            _ => std::cmp::Ordering::Equal,
        });
        out
    }

    pub fn snapshot_world(&self, world: &World) -> WorldSnapshot {
        let mut entities = Vec::new();
        for entity in world.all_entities() {
            let components = self.snapshot_entity(world, entity);
            if !components.is_empty() {
                entities.push(EntitySnapshot { components });
            }
        }
        WorldSnapshot { entities }
    }

    pub fn apply_snapshot(&self, world: &mut World, snap: &WorldSnapshot) -> Result<(), ReflectError> {
        for ent in &snap.entities {
            let e = world.spawn();
            for c in &ent.components {
                self.insert_component(world, e, c)?;
            }
        }
        Ok(())
    }

    pub fn to_ron(&self, world: &World) -> Result<String, ron::Error> {
        ron::ser::to_string_pretty(&self.snapshot_world(world), ron::ser::PrettyConfig::default())
    }

    pub fn from_ron(&self, world: &mut World, ron_src: &str) -> Result<(), ReflectError> {
        let snap: WorldSnapshot =
            ron::from_str(ron_src).map_err(|e| ReflectError::msg(e.to_string()))?;
        self.apply_snapshot(world, &snap)
    }
}

fn insert_typed<T: Reflect + Default>(
    world: &mut World,
    entity: Entity,
    value: &ReflectValue,
) -> Result<(), ReflectError> {
    let mut c = T::default();
    c.apply(value)?;
    world.add_component(entity, c);
    Ok(())
}

fn read_typed<T: Reflect>(world: &World, entity: Entity) -> Option<ReflectValue> {
    world.get_component::<T>(entity).map(|c| c.to_value())
}

fn has_typed<T: Reflect>(world: &World, entity: Entity) -> bool {
    world.get_component::<T>(entity).is_some()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EntitySnapshot {
    pub components: Vec<ReflectValue>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct WorldSnapshot {
    pub entities: Vec<EntitySnapshot>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_math::{Transform, Vec3};

    #[test]
    fn transform_ron_roundtrip() {
        let mut registry = TypeRegistry::new();
        register_engine_defaults(&mut registry);

        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)));

        let ron = registry.to_ron(&world).unwrap();
        assert!(ron.contains("Transform"));
        assert!(ron.contains("1.0") || ron.contains("1"));

        let mut world2 = World::new();
        registry.from_ron(&mut world2, &ron).unwrap();
        let t = world2
            .all_entities()
            .find_map(|id| world2.get_component::<Transform>(id).copied())
            .expect("restored transform");
        assert!((t.translation - Vec3::new(1.0, 2.0, 3.0)).length() < 1e-4);
    }
}
