//! Scripts and the game world: the [`WorldQueries`] that scripts see, script-visible names, tags
//! and values, and the `sys.` commands applied after each script call.
//!
//! [`run_script_update`] runs one `on_update` in four steps: it moves the [`PhysicsContext`] and
//! [`NavWorld`] resources out of the world (the installed queries must own them), snapshots names,
//! tags, positions and [`ScriptValues`], runs the script, moves the resources back and applies the
//! commands the script emitted, in emission order.
//!
//! A command carries its operand in its name, because a script command has only three numbers:
//!
//! * `sys.spawn.<prefab>`: spawns a prefab from [`ScriptPrefabs`] at the position in the arguments;
//! * `sys.despawn.<id>`: despawns the entity with that script id (unknown ids are ignored);
//! * `sys.set_position.<id>`: moves the entity (its [`Transform`] translation) to the arguments;
//! * `sys.set_active.<id>`: `1` removes [`ScriptInactive`], `0` adds it.
//!
//! Script entity ids are the [`Entity::id`] values of live entities. Only the host applies
//! `sys.` commands; the script-side permission (`commands_system`) is checked before they are queued.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use redox_ecs::{Entity, World};
use redox_math::{Quat, Transform, Vec3};
use redox_nav::{AgentClass, NavWorld};
use redox_physics::{PhysicsContext, QueryShape};
use redox_scene::{Name, Tag};
use redox_script::{InstanceId, ScriptEntityId, ScriptError, ScriptRuntime, WorldQueries};

/// Prefix of the commands the host applies after a script call.
pub const SYSTEM_COMMAND_PREFIX: &str = "sys.";

/// Named numeric values of an entity, read by other scripts with `get_value_of`.
#[derive(Clone, Debug, Default)]
pub struct ScriptValues(pub HashMap<String, f32>);

/// Marker added by `sys.set_active.<id>` with argument 0: [`redox_ecs::Inactive`], which the
/// renderer's extraction and the navigation systems skip.
pub use redox_ecs::Inactive as ScriptInactive;

/// Builds one prefab at the given position and returns the new entity.
pub type PrefabSpawn = Box<dyn Fn(&mut World, Vec3) -> Entity + Send + Sync>;

/// Prefabs that scripts may spawn with `sys.spawn.<prefab>`. Inserted into the world as a resource.
#[derive(Default)]
pub struct ScriptPrefabs {
    spawns: HashMap<String, PrefabSpawn>,
}

impl ScriptPrefabs {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers the prefab called `name`, replacing an earlier one with the same name.
    pub fn register<F>(&mut self, name: impl Into<String>, spawn: F)
    where
        F: Fn(&mut World, Vec3) -> Entity + Send + Sync + 'static,
    {
        self.spawns.insert(name.into(), Box::new(spawn));
    }

    /// Whether a prefab with this name is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.spawns.contains_key(name)
    }
}

/// Data for one script call: the moved resources and the snapshot of the ECS data.
#[derive(Default)]
struct Snapshot {
    physics: Option<PhysicsContext>,
    nav: Option<NavWorld>,
    ids_by_name: HashMap<String, u32>,
    ids_by_tag: HashMap<String, Vec<u32>>,
    positions: HashMap<u32, [f32; 3]>,
    values: HashMap<u32, HashMap<String, f32>>,
}

impl Snapshot {
    /// Names, tags, positions and values of every entity that has them.
    fn of(world: &World) -> Self {
        let mut snap = Self::default();
        for (entity, name) in world.query::<(Entity, &Name)>().iter(world) {
            snap.ids_by_name
                .entry(name.0.clone())
                .or_insert(entity.id());
        }
        for (entity, tag) in world.query::<(Entity, &Tag)>().iter(world) {
            snap.ids_by_tag
                .entry(tag.0.clone())
                .or_default()
                .push(entity.id());
        }
        for (entity, transform) in world.query::<(Entity, &Transform)>().iter(world) {
            snap.positions
                .insert(entity.id(), transform.translation.to_array());
        }
        for (entity, values) in world.query::<(Entity, &ScriptValues)>().iter(world) {
            snap.values.insert(entity.id(), values.0.clone());
        }
        snap
    }
}

/// The [`WorldQueries`] installed for one call. It answers from the shared [`Snapshot`]; once
/// [`run_script_update`] has taken the physics and navigation back, it answers misses.
struct WorldAdapter {
    shared: Arc<Mutex<Snapshot>>,
}

impl WorldAdapter {
    fn lock(&self) -> MutexGuard<'_, Snapshot> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Appends at most `limit` ids in total (counting those already in `out`).
fn push_ids(out: &mut Vec<ScriptEntityId>, ids: impl Iterator<Item = u32>, limit: usize) {
    let room = limit.saturating_sub(out.len());
    out.extend(ids.take(room).map(ScriptEntityId));
}

impl WorldQueries for WorldAdapter {
    fn raycast(&mut self, origin: [f32; 3], direction: [f32; 3], max_distance: f32) -> Option<f32> {
        let snap = self.lock();
        let physics = snap.physics.as_ref()?;
        let origin = Vec3::from_array(origin);
        let (point, _, _) =
            physics.cast_ray_with_normal(origin, Vec3::from_array(direction), max_distance, &[])?;
        Some(point.distance(origin))
    }

    fn raycast_entity(
        &mut self,
        origin: [f32; 3],
        direction: [f32; 3],
        max_distance: f32,
    ) -> Option<(ScriptEntityId, f32)> {
        let snap = self.lock();
        let physics = snap.physics.as_ref()?;
        let origin = Vec3::from_array(origin);
        let (point, _, entity) =
            physics.cast_ray_with_normal(origin, Vec3::from_array(direction), max_distance, &[])?;
        Some((ScriptEntityId(entity?.id()), point.distance(origin)))
    }

    fn overlap_sphere(
        &mut self,
        center: [f32; 3],
        radius: f32,
        out: &mut Vec<ScriptEntityId>,
        limit: usize,
    ) {
        let snap = self.lock();
        let Some(physics) = snap.physics.as_ref() else {
            return;
        };
        let hits = physics.overlap_shape(
            QueryShape::Sphere { radius },
            Vec3::from_array(center),
            Quat::IDENTITY,
            &[],
            u32::MAX,
        );
        push_ids(out, hits.iter().map(|e| e.id()), limit);
    }

    fn overlap_box(
        &mut self,
        center: [f32; 3],
        half_extent: [f32; 3],
        out: &mut Vec<ScriptEntityId>,
        limit: usize,
    ) {
        let snap = self.lock();
        let Some(physics) = snap.physics.as_ref() else {
            return;
        };
        let hits = physics.overlap_shape(
            QueryShape::Cuboid {
                half_extents: Vec3::from_array(half_extent),
            },
            Vec3::from_array(center),
            Quat::IDENTITY,
            &[],
            u32::MAX,
        );
        push_ids(out, hits.iter().map(|e| e.id()), limit);
    }

    fn find_by_name(&mut self, name: &str) -> Option<ScriptEntityId> {
        self.lock()
            .ids_by_name
            .get(name)
            .map(|id| ScriptEntityId(*id))
    }

    fn find_by_tag(&mut self, tag: &str, out: &mut Vec<ScriptEntityId>, limit: usize) {
        let snap = self.lock();
        if let Some(ids) = snap.ids_by_tag.get(tag) {
            push_ids(out, ids.iter().copied(), limit);
        }
    }

    fn position_of(&mut self, id: ScriptEntityId) -> Option<[f32; 3]> {
        self.lock().positions.get(&id.0).copied()
    }

    fn get_value(&mut self, id: ScriptEntityId, key: &str) -> Option<f32> {
        self.lock()
            .values
            .get(&id.0)
            .and_then(|values| values.get(key))
            .copied()
    }

    fn nav_path(
        &mut self,
        from: [f32; 3],
        to: [f32; 3],
        out: &mut Vec<[f32; 3]>,
        limit: usize,
    ) -> bool {
        let snap = self.lock();
        let Some(nav) = snap.nav.as_ref() else {
            return false;
        };
        // Agent class 0: the default class of the baked data.
        let Some(path) = nav.find_path(Vec3::from_array(from), Vec3::from_array(to), AgentClass(0))
        else {
            return false;
        };
        let found = !path.is_empty();
        let room = limit.saturating_sub(out.len());
        out.extend(path.iter().take(room).map(|p| p.to_array()));
        found
    }
}

/// Runs `on_update` of instance `id` against `world`, then applies the `sys.` commands it emitted
/// in emission order.
///
/// The [`PhysicsContext`] and [`NavWorld`] resources are absent from `world` while the script runs
/// and are put back afterwards, also when the update fails. The result of the update is returned
/// after the commands have been applied.
pub fn run_script_update(
    world: &mut World,
    runtime: &mut ScriptRuntime,
    id: InstanceId,
    dt: f32,
) -> Result<(), ScriptError> {
    let mut snap = Snapshot::of(world);
    snap.physics = world.remove_resource::<PhysicsContext>();
    snap.nav = world.remove_resource::<NavWorld>();
    let shared = Arc::new(Mutex::new(snap));
    runtime.set_world_queries(
        id,
        Box::new(WorldAdapter {
            shared: Arc::clone(&shared),
        }),
    );
    let result = runtime.update(id, dt);

    let (physics, nav) = {
        let mut snap = shared.lock().unwrap_or_else(PoisonError::into_inner);
        (snap.physics.take(), snap.nav.take())
    };
    if let Some(physics) = physics {
        world.insert_resource(physics);
    }
    if let Some(nav) = nav {
        world.insert_resource(nav);
    }
    for command in runtime.take_commands(id) {
        apply_command(world, &command.name, command.args);
    }
    result
}

/// Applies one emitted command. Names without the `sys.` prefix belong to the game and are ignored.
/// Malformed or unknown `sys.` commands are logged and skipped.
pub fn apply_command(world: &mut World, name: &str, args: [f32; 3]) {
    let Some(operation) = name.strip_prefix(SYSTEM_COMMAND_PREFIX) else {
        return;
    };
    let position = Vec3::from_array(args);
    if let Some(prefab) = operation.strip_prefix("spawn.") {
        spawn_prefab(world, prefab, position);
    } else if let Some(target) = operation.strip_prefix("despawn.") {
        with_target(world, name, target, |world, entity| world.despawn(entity));
    } else if let Some(target) = operation.strip_prefix("set_position.") {
        with_target(world, name, target, move |world, entity| {
            match world.get_component_mut::<Transform>(entity) {
                Some(transform) => transform.translation = position,
                None => log::warn!("set_position: entity {} has no Transform", entity.id()),
            }
        });
    } else if let Some(target) = operation.strip_prefix("set_active.") {
        let active = args[0] >= 0.5;
        with_target(world, name, target, move |world, entity| {
            let inactive = world.get_component::<ScriptInactive>(entity).is_some();
            if active && inactive {
                let _ = world.remove_component::<ScriptInactive>(entity);
            } else if !active && !inactive {
                world.add_component(entity, ScriptInactive);
            }
        });
    } else {
        log::warn!("unknown system command `{name}` ignored");
    }
}

/// Spawns the prefab at `position`. The table is taken out of the world while the spawn function
/// runs, so the function may borrow the world mutably.
fn spawn_prefab(world: &mut World, prefab: &str, position: Vec3) {
    let Some(prefabs) = world.remove_resource::<ScriptPrefabs>() else {
        log::warn!("no ScriptPrefabs resource; cannot spawn `{prefab}`");
        return;
    };
    let spawned = prefabs
        .spawns
        .get(prefab)
        .map(|spawn| spawn(world, position));
    if spawned.is_none() {
        log::warn!("unknown prefab `{prefab}` ignored");
    }
    world.insert_resource(prefabs);
}

/// Runs `act` on the live entity whose script id is the decimal number `target`. Malformed or
/// unknown ids are logged and skipped.
fn with_target(
    world: &mut World,
    command: &str,
    target: &str,
    act: impl FnOnce(&mut World, Entity),
) {
    let Ok(id) = target.parse::<u32>() else {
        log::warn!("command `{command}` has no valid entity id");
        return;
    };
    let found = world.all_entities().find(|entity| entity.id() == id);
    match found {
        Some(entity) => act(world, entity),
        None => log::debug!("command `{command}` ignores unknown entity {id}"),
    }
}
