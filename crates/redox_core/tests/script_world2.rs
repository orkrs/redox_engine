//! Host API v2 against the game world: queries answered from the physics and ECS data, and `sys.`
//! commands applied after the script call (`redox_core::script_world`).

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_core::script_world::{ScriptInactive, ScriptPrefabs, ScriptValues, run_script_update};
use redox_ecs::{Entity, World};
use redox_math::{Quat, Transform, Vec3};
use redox_nav::{AgentParams, NavAreaId, NavData, NavPolygon, NavTile, NavWorld};
use redox_physics::PhysicsContext;
use redox_scene::{Name, Tag};
use redox_script::{ApiPermissions, InstanceId, ScriptConfig, ScriptRuntime};

fn module(text: &str) -> Vec<u8> {
    wat::parse_str(text).expect("valid wat")
}

/// A module that emits `commands` (name and three numbers) on every update, in order.
fn commands_module(commands: &[(String, [f32; 3])]) -> Vec<u8> {
    let mut data = String::new();
    let mut calls = String::new();
    for (i, (name, args)) in commands.iter().enumerate() {
        let offset = i * 32;
        data.push_str(&format!(
            "  (data (i32.const {offset}) \"{name}\")\n",
            offset = offset,
            name = name
        ));
        calls.push_str(&format!(
            "    (call $cmd (i32.const {offset}) (i32.const {len}) (f32.const {a}) (f32.const {b}) (f32.const {c}))\n",
            offset = offset,
            len = name.len(),
            a = args[0],
            b = args[1],
            c = args[2]
        ));
    }
    module(&format!(
        "(module\n  (import \"env\" \"emit_command\" (func $cmd (param i32 i32 f32 f32 f32)))\n  (memory (export \"memory\") 1)\n{data}  (func (export \"on_update\") (param f32)\n{calls}  ))\n"
    ))
}

/// A world with a physics context and no entities.
fn physics_world() -> World {
    let mut world = World::new();
    world.insert_resource(PhysicsContext::new(Vec3::ZERO));
    world
}

/// Spawns an entity with a fixed ball of radius 0.5 m at `at`, then steps the physics once so the
/// query pipeline sees it.
fn place_ball(world: &mut World, at: Vec3) -> Entity {
    let entity = world.spawn();
    let mut physics = world.remove_resource::<PhysicsContext>().unwrap();
    physics
        .add_rigid_body(
            entity,
            RigidBodyBuilder::fixed().translation(at),
            vec![ColliderBuilder::ball(0.5)],
        )
        .unwrap();
    physics.step(1.0 / 60.0);
    world.insert_resource(physics);
    entity
}

fn run(world: &mut World, rt: &mut ScriptRuntime, id: InstanceId) {
    run_script_update(world, rt, id, 0.016).unwrap();
}

/// Overlaps a sphere of radius 2 m at the origin; stores the count as "n" and the first id as
/// "first".
fn overlap_module() -> Vec<u8> {
    module(
        r#"(module
  (import "env" "overlap_sphere" (func $sphere (param f32 f32 f32 f32 i32 i32) (result i32)))
  (import "env" "set_f32" (func $set (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "n")
  (data (i32.const 8) "first")
  (func (export "on_update") (param f32)
    (call $set (i32.const 0) (i32.const 1)
      (f32.convert_i32_s (call $sphere (f32.const 0) (f32.const 0) (f32.const 0) (f32.const 2) (i32.const 400) (i32.const 4))))
    (call $set (i32.const 8) (i32.const 5) (f32.convert_i32_s (i32.load (i32.const 400))))))"#,
    )
}

#[test]
fn overlap_sphere_returns_the_entities_inside() {
    let mut world = physics_world();
    let inside = place_ball(&mut world, Vec3::new(1.0, 0.0, 0.0));
    let mut rt = ScriptRuntime::new(ScriptConfig::default());
    let id = rt.instantiate(&overlap_module()).unwrap();
    run(&mut world, &mut rt, id);
    assert_eq!(rt.read_output(id, "first"), Some(inside.id() as f32));
}

#[test]
fn overlap_sphere_ignores_entities_outside_the_radius() {
    let mut world = physics_world();
    let inside = place_ball(&mut world, Vec3::new(1.0, 0.0, 0.0));
    let _outside = place_ball(&mut world, Vec3::new(10.0, 0.0, 0.0));
    let mut rt = ScriptRuntime::new(ScriptConfig::default());
    let id = rt.instantiate(&overlap_module()).unwrap();
    run(&mut world, &mut rt, id);
    assert_eq!(rt.read_output(id, "n"), Some(1.0));
    assert_eq!(rt.read_output(id, "first"), Some(inside.id() as f32));
}

/// Reads the named value "hp" of the entity whose id is the input "target".
const VALUE_READER: &str = r#"(module
  (import "env" "get_f32" (func $get (param i32 i32) (result f32)))
  (import "env" "get_value_of" (func $value (param i32 i32 i32 i32) (result i32)))
  (import "env" "set_f32" (func $set (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "target")
  (data (i32.const 8) "hp")
  (data (i32.const 16) "hp_out")
  (func (export "on_update") (param f32)
    (drop (call $value (i32.trunc_f32_s (call $get (i32.const 0) (i32.const 6))) (i32.const 8) (i32.const 2) (i32.const 500)))
    (call $set (i32.const 16) (i32.const 6) (f32.load (i32.const 500)))))"#;

#[test]
fn get_value_of_reads_another_entities_named_value() {
    let mut world = physics_world();
    let other = world.spawn();
    let mut values = ScriptValues::default();
    values.0.insert("hp".to_owned(), 80.0);
    world.add_component(other, values);
    let mut rt = ScriptRuntime::new(ScriptConfig::default());
    let id = rt.instantiate(&module(VALUE_READER)).unwrap();
    rt.set_input(id, "target", other.id() as f32);
    run(&mut world, &mut rt, id);
    assert_eq!(rt.read_output(id, "hp_out"), Some(80.0));
}

/// Asks for a path from (0,0,0) to (5,0,0) and stores the number of points as "n".
const PATH_QUERY: &str = r#"(module
  (import "env" "nav_path" (func $path (param i32 i32 i32 i32) (result i32)))
  (import "env" "set_f32" (func $set (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "n")
  (func (export "on_update") (param f32)
    (f32.store (i32.const 100) (f32.const 0))
    (f32.store (i32.const 104) (f32.const 0))
    (f32.store (i32.const 108) (f32.const 0))
    (f32.store (i32.const 112) (f32.const 5))
    (f32.store (i32.const 116) (f32.const 0))
    (f32.store (i32.const 120) (f32.const 0))
    (call $set (i32.const 0) (i32.const 1)
      (f32.convert_i32_s (call $path (i32.const 100) (i32.const 112) (i32.const 200) (i32.const 32))))))"#;

#[test]
fn nav_path_is_empty_without_a_path() {
    let mut world = physics_world();
    let mut rt = ScriptRuntime::new(ScriptConfig::default());
    let id = rt.instantiate(&module(PATH_QUERY)).unwrap();
    run(&mut world, &mut rt, id);
    assert_eq!(rt.read_output(id, "n"), Some(0.0), "no NavWorld resource");
}

/// Looks up "player" 70 times and stores how many lookups answered as "hits".
const LOOKUP_LOOP: &str = r#"(module
  (import "env" "find_by_name" (func $find (param i32 i32) (result i32)))
  (import "env" "set_f32" (func $set (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "player")
  (data (i32.const 8) "hits")
  (func (export "on_update") (param f32) (local $i i32) (local $hits f32)
    (loop $again
      (if (i32.ge_s (call $find (i32.const 0) (i32.const 6)) (i32.const 0))
        (then (local.set $hits (f32.add (local.get $hits) (f32.const 1)))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $again (i32.lt_u (local.get $i) (i32.const 70))))
    (call $set (i32.const 8) (i32.const 4) (local.get $hits))))"#;

#[test]
fn queries_are_counted_against_the_per_call_limit() {
    let mut world = physics_world();
    let player = world.spawn();
    world.add_component(player, Name("player".to_owned()));
    let mut rt = ScriptRuntime::new(ScriptConfig::default());
    let id = rt.instantiate(&module(LOOKUP_LOOP)).unwrap();
    run(&mut world, &mut rt, id);
    assert_eq!(rt.read_output(id, "hits"), Some(64.0));
}

/// Spawns a "crate" at the arguments' position; its entity carries the name "crate".
fn crate_prefabs() -> ScriptPrefabs {
    let mut prefabs = ScriptPrefabs::new();
    prefabs.register("crate", |world: &mut World, position: Vec3| {
        let entity = world.spawn();
        world.add_component(
            entity,
            Transform {
                translation: position,
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
        );
        world.add_component(entity, Name("crate".to_owned()));
        entity
    });
    prefabs
}

fn crates(world: &World) -> usize {
    world
        .query::<(Entity, &Name)>()
        .iter(world)
        .filter(|(_, n)| n.0 == "crate")
        .count()
}

#[test]
fn spawn_happens_after_the_call() {
    let mut world = physics_world();
    world.insert_resource(crate_prefabs());
    let mut rt = ScriptRuntime::new(ScriptConfig::default());
    let id = rt
        .instantiate(&commands_module(&[(
            "sys.spawn.crate".to_owned(),
            [1.0, 2.0, 3.0],
        )]))
        .unwrap();
    assert_eq!(crates(&world), 0);
    run(&mut world, &mut rt, id);
    assert_eq!(crates(&world), 1);
    let spawned = world
        .query::<(&Name, &Transform)>()
        .iter(&world)
        .find(|(n, _)| n.0 == "crate")
        .map(|(_, t)| t.translation)
        .unwrap();
    assert_eq!(spawned, Vec3::new(1.0, 2.0, 3.0));
}

#[test]
fn commands_are_applied_in_emission_order() {
    let mut world = physics_world();
    let mover = world.spawn();
    world.add_component(
        mover,
        Transform {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        },
    );
    let id_text = mover.id();
    let mut rt = ScriptRuntime::new(ScriptConfig::default());
    let id = rt
        .instantiate(&commands_module(&[
            (format!("sys.set_position.{id_text}"), [1.0, 0.0, 0.0]),
            (format!("sys.set_position.{id_text}"), [5.0, 0.0, 0.0]),
        ]))
        .unwrap();
    run(&mut world, &mut rt, id);
    let position = world.get_component::<Transform>(mover).unwrap().translation;
    assert_eq!(position, Vec3::new(5.0, 0.0, 0.0), "the later command wins");
}

#[test]
fn an_unknown_prefab_is_logged_and_ignored() {
    let mut world = physics_world();
    world.insert_resource(crate_prefabs());
    let before = world.all_entities().count();
    let mut rt = ScriptRuntime::new(ScriptConfig::default());
    let id = rt
        .instantiate(&commands_module(&[("sys.spawn.nope".to_owned(), [0.0; 3])]))
        .unwrap();
    run(&mut world, &mut rt, id);
    assert_eq!(world.all_entities().count(), before);
    assert!(
        !rt.is_disabled(id),
        "a bad command does not disable the script"
    );
}

#[test]
fn system_commands_require_the_permission() {
    let mut world = physics_world();
    world.insert_resource(crate_prefabs());
    let config = ScriptConfig {
        api: ApiPermissions {
            commands_system: false,
            ..ApiPermissions::all()
        },
        ..ScriptConfig::default()
    };
    let mut rt = ScriptRuntime::new(config);
    let id = rt
        .instantiate(&commands_module(&[(
            "sys.spawn.crate".to_owned(),
            [0.0; 3],
        )]))
        .unwrap();
    run(&mut world, &mut rt, id);
    assert_eq!(crates(&world), 0, "dropped by the script side");
}

#[test]
fn despawn_of_an_unknown_id_is_ignored() {
    let mut world = physics_world();
    let keep = world.spawn();
    // `all_entities` lists entities that own components.
    world.add_component(keep, Name("keep".to_owned()));
    let before = world.all_entities().count();
    let mut rt = ScriptRuntime::new(ScriptConfig::default());
    let id = rt
        .instantiate(&commands_module(&[(
            "sys.despawn.4000000".to_owned(),
            [0.0; 3],
        )]))
        .unwrap();
    run(&mut world, &mut rt, id);
    assert_eq!(world.all_entities().count(), before);
    assert!(world.all_entities().any(|e| e == keep));
}

#[test]
fn set_active_toggles_the_inactive_marker() {
    let mut world = physics_world();
    let target = world.spawn();
    world.add_component(target, Name("target".to_owned()));
    let id_text = target.id();
    let mut rt = ScriptRuntime::new(ScriptConfig::default());
    let off = rt
        .instantiate(&commands_module(&[(
            format!("sys.set_active.{id_text}"),
            [0.0; 3],
        )]))
        .unwrap();
    run(&mut world, &mut rt, off);
    assert!(world.get_component::<ScriptInactive>(target).is_some());

    let on = rt
        .instantiate(&commands_module(&[(
            format!("sys.set_active.{id_text}"),
            [1.0, 0.0, 0.0],
        )]))
        .unwrap();
    run(&mut world, &mut rt, on);
    assert!(world.get_component::<ScriptInactive>(target).is_none());
}

#[test]
fn a_switched_off_entity_is_not_found_by_name() {
    let mut world = physics_world();
    let ghost = world.spawn();
    world.add_component(ghost, Name("ghost".to_owned()));
    world.add_component(ghost, ScriptInactive);
    let mut rt = ScriptRuntime::new(ScriptConfig::default());
    let id = rt
        .instantiate(&module(
            r#"(module
  (import "env" "find_by_name" (func $find (param i32 i32) (result i32)))
  (import "env" "set_f32" (func $set (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "ghost")
  (data (i32.const 16) "found")
  (func (export "on_update") (param f32)
    (call $set (i32.const 16) (i32.const 5)
      (f32.convert_i32_s (call $find (i32.const 0) (i32.const 5))))))"#,
        ))
        .unwrap();
    run(&mut world, &mut rt, id);
    assert_eq!(rt.read_output(id, "found"), Some(-1.0), "switched off");

    world.remove_component::<ScriptInactive>(ghost);
    run(&mut world, &mut rt, id);
    let found = rt.read_output(id, "found").unwrap();
    assert!(found >= 0.0, "switched on again, got {found}");
}

#[test]
fn physics_is_back_in_the_world_after_the_call() {
    let mut world = physics_world();
    let mut rt = ScriptRuntime::new(ScriptConfig::default());
    let id = rt.instantiate(&overlap_module()).unwrap();
    run(&mut world, &mut rt, id);
    assert!(world.get_resource::<PhysicsContext>().is_some());
}

#[test]
fn tags_are_visible_to_scripts_through_the_snapshot() {
    let mut world = physics_world();
    let enemy = world.spawn();
    world.add_component(enemy, Tag("enemy".to_owned()));
    let mut rt = ScriptRuntime::new(ScriptConfig::default());
    let id = rt
        .instantiate(&module(
            r#"(module
  (import "env" "find_by_tag" (func $tag (param i32 i32 i32 i32) (result i32)))
  (import "env" "set_f32" (func $set (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "enemy")
  (data (i32.const 8) "count")
  (func (export "on_update") (param f32)
    (call $set (i32.const 8) (i32.const 5)
      (f32.convert_i32_s (call $tag (i32.const 0) (i32.const 5) (i32.const 400) (i32.const 4))))))"#,
        ))
        .unwrap();
    run(&mut world, &mut rt, id);
    assert_eq!(rt.read_output(id, "count"), Some(1.0));
}

/// One tile of `cols` x 2 cells (1 m each) along +X, baked as navigation data.
fn corridor(cols: u16) -> NavData {
    let mut vertices = Vec::new();
    for r in 0..=1u16 {
        for c in 0..=cols {
            vertices.push([f32::from(c), 0.0, f32::from(r)]);
        }
    }
    let stride = cols + 1;
    let polygons = (0..cols)
        .map(|c| NavPolygon {
            indices: vec![c, stride + c, stride + c + 1, c + 1],
            area: NavAreaId::WALKABLE,
        })
        .collect();
    NavData {
        agent: AgentParams::default(),
        tile_size: 16.0,
        tiles: vec![NavTile {
            coord: (0, 0),
            vertices,
            polygons,
        }],
    }
}

#[test]
fn nav_path_returns_a_polyline_on_a_baked_navmesh() {
    let mut world = physics_world();
    world.insert_resource(NavWorld::from_data(&corridor(8)).unwrap());
    let mut rt = ScriptRuntime::new(ScriptConfig::default());
    let id = rt
        .instantiate(&module(
            r#"(module
  (import "env" "nav_path" (func $path (param i32 i32 i32 i32) (result i32)))
  (import "env" "set_f32" (func $set (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "count")
  (data (i32.const 16) "last_x")
  (func (export "on_update") (param f32) (local $n i32)
    ;; from (0.5, 0, 0.5) at 600, to (7.5, 0, 0.5) at 612
    (f32.store (i32.const 600) (f32.const 0.5))
    (f32.store (i32.const 604) (f32.const 0))
    (f32.store (i32.const 608) (f32.const 0.5))
    (f32.store (i32.const 612) (f32.const 7.5))
    (f32.store (i32.const 616) (f32.const 0))
    (f32.store (i32.const 620) (f32.const 0.5))
    (local.set $n (call $path (i32.const 600) (i32.const 612) (i32.const 700) (i32.const 8)))
    (call $set (i32.const 0) (i32.const 5) (f32.convert_i32_s (local.get $n)))
    (call $set (i32.const 16) (i32.const 6)
      (f32.load (i32.add (i32.const 700) (i32.mul (i32.sub (local.get $n) (i32.const 1)) (i32.const 12)))))))"#,
        ))
        .unwrap();
    run(&mut world, &mut rt, id);
    let count = rt.read_output(id, "count").unwrap();
    assert!(count >= 2.0, "a path of at least two points, got {count}");
    let last_x = rt.read_output(id, "last_x").unwrap();
    assert!(
        (last_x - 7.5).abs() < 0.1,
        "the path ends at the goal, got {last_x}"
    );
}
