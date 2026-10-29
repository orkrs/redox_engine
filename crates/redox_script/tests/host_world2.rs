//! Host API v2, step 2: entity lookup, positions and entity ray hits (`find_by_name`,
//! `find_by_tag`, `position_of`, `raycast_entity`). Every test runs on both backends.

#![allow(clippy::unwrap_used)]

use redox_script::{
    ApiPermissions, BackendKind, ScriptConfig, ScriptEntityId, ScriptError, ScriptRuntime,
    WorldQueries,
};

fn wasm(text: &str) -> Vec<u8> {
    wat::parse_str(text).expect("valid wat")
}

fn backends() -> Vec<BackendKind> {
    #[cfg_attr(not(feature = "wasmtime"), allow(unused_mut))]
    let mut kinds = vec![BackendKind::Wasmi];
    #[cfg(feature = "wasmtime")]
    kinds.push(BackendKind::Wasmtime);
    kinds
}

fn runtime(kind: BackendKind, config: ScriptConfig) -> ScriptRuntime {
    ScriptRuntime::with_backend(config, kind).unwrap()
}

/// A small scene: one named entity, a tag shared by five entities, one positioned entity and a
/// target in front of any ray that starts near the origin.
struct Scene;

impl WorldQueries for Scene {
    fn raycast(&mut self, _: [f32; 3], _: [f32; 3], _: f32) -> Option<f32> {
        None
    }

    fn raycast_entity(
        &mut self,
        origin: [f32; 3],
        _direction: [f32; 3],
        _max_distance: f32,
    ) -> Option<(ScriptEntityId, f32)> {
        (origin[0] < 1.0).then_some((ScriptEntityId(9), 4.0))
    }

    fn find_by_name(&mut self, name: &str) -> Option<ScriptEntityId> {
        (name == "player").then_some(ScriptEntityId(7))
    }

    /// Ignores `limit` on purpose: the host must cut the answer down itself.
    fn find_by_tag(&mut self, tag: &str, out: &mut Vec<ScriptEntityId>, _limit: usize) {
        if tag == "enemy" {
            out.extend((1..=5).map(ScriptEntityId));
        }
    }

    fn position_of(&mut self, id: ScriptEntityId) -> Option<[f32; 3]> {
        (id == ScriptEntityId(4)).then_some([1.5, -2.0, 3.25])
    }
}

/// Looks up "player" (id 7) and an unknown "ghost" (-1), storing both as outputs.
const NAME_LOOKUP: &str = r#"
(module
  (import "env" "find_by_name" (func $find (param i32 i32) (result i32)))
  (import "env" "set_f32" (func $set (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "id")
  (data (i32.const 8) "player")
  (data (i32.const 32) "ghost")
  (data (i32.const 48) "miss")
  (func (export "on_update") (param f32)
    (call $set (i32.const 0) (i32.const 2)
      (f32.convert_i32_s (call $find (i32.const 8) (i32.const 6))))
    (call $set (i32.const 48) (i32.const 4)
      (f32.convert_i32_s (call $find (i32.const 32) (i32.const 5))))))
"#;

#[test]
fn a_script_finds_an_entity_by_name() {
    for kind in backends() {
        let mut rt = runtime(kind, ScriptConfig::default());
        let id = rt.instantiate(&wasm(NAME_LOOKUP)).unwrap();
        rt.set_world_queries(id, Box::new(Scene));
        rt.update(id, 0.016).unwrap();
        assert_eq!(rt.read_output(id, "id"), Some(7.0), "{kind:?}");
    }
}

#[test]
fn an_unknown_name_returns_minus_one() {
    for kind in backends() {
        let mut rt = runtime(kind, ScriptConfig::default());
        let id = rt.instantiate(&wasm(NAME_LOOKUP)).unwrap();
        rt.set_world_queries(id, Box::new(Scene));
        rt.update(id, 0.016).unwrap();
        assert_eq!(rt.read_output(id, "miss"), Some(-1.0), "{kind:?}");
    }
}

/// Asks for at most 3 entities tagged "enemy" into the buffer at 100, then reads four slots.
const TAG_LOOKUP: &str = r#"
(module
  (import "env" "find_by_tag" (func $tag (param i32 i32 i32 i32) (result i32)))
  (import "env" "set_f32" (func $set (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 8) "enemy")
  (data (i32.const 40) "count")
  (data (i32.const 56) "a")
  (data (i32.const 60) "b")
  (data (i32.const 64) "c")
  (data (i32.const 68) "d")
  (func (export "on_update") (param f32)
    (call $set (i32.const 40) (i32.const 5)
      (f32.convert_i32_s (call $tag (i32.const 8) (i32.const 5) (i32.const 100) (i32.const 3))))
    (call $set (i32.const 56) (i32.const 1) (f32.convert_i32_s (i32.load (i32.const 100))))
    (call $set (i32.const 60) (i32.const 1) (f32.convert_i32_s (i32.load (i32.const 104))))
    (call $set (i32.const 64) (i32.const 1) (f32.convert_i32_s (i32.load (i32.const 108))))
    (call $set (i32.const 68) (i32.const 1) (f32.convert_i32_s (i32.load (i32.const 112))))))
"#;

#[test]
fn find_by_tag_fills_the_buffer_up_to_its_capacity() {
    for kind in backends() {
        let mut rt = runtime(kind, ScriptConfig::default());
        let id = rt.instantiate(&wasm(TAG_LOOKUP)).unwrap();
        rt.set_world_queries(id, Box::new(Scene));
        rt.update(id, 0.016).unwrap();
        assert_eq!(rt.read_output(id, "count"), Some(3.0), "{kind:?}");
        assert_eq!(rt.read_output(id, "a"), Some(1.0), "{kind:?}");
        assert_eq!(rt.read_output(id, "b"), Some(2.0), "{kind:?}");
        assert_eq!(rt.read_output(id, "c"), Some(3.0), "{kind:?}");
        assert_eq!(rt.read_output(id, "d"), Some(0.0), "{kind:?}: no fourth id");
    }
}

/// Reads the position of entity 4 into x, y, z, and the status of an unknown id 99 ("none").
/// Writing to a buffer outside memory (70000) is refused and reported as "bad" = 0.
const POSITION: &str = r#"
(module
  (import "env" "position_of" (func $pos (param i32 i32) (result i32)))
  (import "env" "set_f32" (func $set (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "ok")
  (data (i32.const 8) "x")
  (data (i32.const 16) "y")
  (data (i32.const 24) "z")
  (data (i32.const 32) "none")
  (data (i32.const 40) "bad")
  (func (export "on_update") (param f32)
    (call $set (i32.const 0) (i32.const 2)
      (f32.convert_i32_s (call $pos (i32.const 4) (i32.const 200))))
    (call $set (i32.const 8) (i32.const 1) (f32.load (i32.const 200)))
    (call $set (i32.const 16) (i32.const 1) (f32.load (i32.const 204)))
    (call $set (i32.const 24) (i32.const 1) (f32.load (i32.const 208)))
    (call $set (i32.const 32) (i32.const 4)
      (f32.convert_i32_s (call $pos (i32.const 99) (i32.const 300))))
    (call $set (i32.const 40) (i32.const 3)
      (f32.convert_i32_s (call $pos (i32.const 4) (i32.const 70000))))))
"#;

#[test]
fn position_of_writes_three_floats() {
    for kind in backends() {
        let mut rt = runtime(kind, ScriptConfig::default());
        let id = rt.instantiate(&wasm(POSITION)).unwrap();
        rt.set_world_queries(id, Box::new(Scene));
        rt.update(id, 0.016).unwrap();
        assert_eq!(rt.read_output(id, "ok"), Some(1.0), "{kind:?}");
        assert_eq!(rt.read_output(id, "x"), Some(1.5), "{kind:?}");
        assert_eq!(rt.read_output(id, "y"), Some(-2.0), "{kind:?}");
        assert_eq!(rt.read_output(id, "z"), Some(3.25), "{kind:?}");
        assert_eq!(rt.read_output(id, "none"), Some(0.0), "{kind:?}: unknown id");
    }
}

#[test]
fn a_pointer_outside_memory_is_refused_without_a_trap() {
    for kind in backends() {
        let mut rt = runtime(kind, ScriptConfig::default());
        let id = rt.instantiate(&wasm(POSITION)).unwrap();
        rt.set_world_queries(id, Box::new(Scene));
        rt.update(id, 0.016).unwrap();
        assert_eq!(rt.read_output(id, "bad"), Some(0.0), "{kind:?}");
        assert!(!rt.is_disabled(id), "{kind:?}: the instance keeps running");
    }
}

/// Casts a ray from (0, 1, 0) straight down with a 10 m limit; stores hit, id and distance.
const ENTITY_RAY: &str = r#"
(module
  (import "env" "raycast_entity" (func $hit (param i32 i32 f32 i32) (result i32)))
  (import "env" "set_f32" (func $set (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "hit")
  (data (i32.const 8) "id")
  (data (i32.const 16) "dist")
  (func (export "on_update") (param f32)
    (f32.store (i32.const 300) (f32.const 0))
    (f32.store (i32.const 304) (f32.const 1))
    (f32.store (i32.const 308) (f32.const 0))
    (f32.store (i32.const 312) (f32.const 0))
    (f32.store (i32.const 316) (f32.const -1))
    (f32.store (i32.const 320) (f32.const 0))
    (call $set (i32.const 0) (i32.const 3)
      (f32.convert_i32_s (call $hit (i32.const 300) (i32.const 312) (f32.const 10) (i32.const 340))))
    (call $set (i32.const 8) (i32.const 2) (f32.convert_i32_s (i32.load (i32.const 340))))
    (call $set (i32.const 16) (i32.const 4) (f32.load (i32.const 344)))))
"#;

#[test]
fn raycast_entity_returns_id_and_distance() {
    for kind in backends() {
        let mut rt = runtime(kind, ScriptConfig::default());
        let id = rt.instantiate(&wasm(ENTITY_RAY)).unwrap();
        rt.set_world_queries(id, Box::new(Scene));
        rt.update(id, 0.016).unwrap();
        assert_eq!(rt.read_output(id, "hit"), Some(1.0), "{kind:?}");
        assert_eq!(rt.read_output(id, "id"), Some(9.0), "{kind:?}");
        assert_eq!(rt.read_output(id, "dist"), Some(4.0), "{kind:?}");
    }
}

#[test]
fn without_the_query_permission_the_import_is_missing_or_denied() {
    for kind in backends() {
        let config = ScriptConfig {
            api: ApiPermissions {
                query: false,
                ..ApiPermissions::all()
            },
            ..ScriptConfig::default()
        };
        let mut rt = runtime(kind, config);
        assert!(
            matches!(rt.instantiate(&wasm(NAME_LOOKUP)), Err(ScriptError::Link(_))),
            "{kind:?}: find_by_name needs `query`"
        );
        assert!(matches!(
            rt.instantiate(&wasm(TAG_LOOKUP)),
            Err(ScriptError::Link(_))
        ));
    }
}

/// A field with two entities 1 m apart, a named value on entity 7 and a straight walkable path
/// that exists between points with a positive x coordinate.
struct Field;

impl WorldQueries for Field {
    fn raycast(&mut self, _: [f32; 3], _: [f32; 3], _: f32) -> Option<f32> {
        None
    }

    /// Entities 1 and 2 lie within 2 m of the centre; a larger radius is not needed here.
    fn overlap_sphere(
        &mut self,
        _center: [f32; 3],
        radius: f32,
        out: &mut Vec<ScriptEntityId>,
        _limit: usize,
    ) {
        if radius >= 1.0 {
            out.extend([ScriptEntityId(1), ScriptEntityId(2)]);
        }
    }

    fn get_value(&mut self, id: ScriptEntityId, key: &str) -> Option<f32> {
        (id == ScriptEntityId(7) && key == "hp").then_some(80.0)
    }

    fn nav_path(
        &mut self,
        from: [f32; 3],
        to: [f32; 3],
        out: &mut Vec<[f32; 3]>,
        _limit: usize,
    ) -> bool {
        if from[0] < 0.0 {
            return false;
        }
        out.extend([from, to]);
        true
    }
}

/// Counts the entities in a sphere of radius 2 into "n" and their first id into "first";
/// reads the hp of entity 7 into "hp" and the hp of entity 8 (missing) into "none".
const FIELD_QUERIES: &str = r#"
(module
  (import "env" "overlap_sphere" (func $sphere (param f32 f32 f32 f32 i32 i32) (result i32)))
  (import "env" "get_value_of" (func $value (param i32 i32 i32 i32) (result i32)))
  (import "env" "set_f32" (func $set (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "n")
  (data (i32.const 8) "first")
  (data (i32.const 16) "hp")
  (data (i32.const 24) "none")
  (data (i32.const 40) "hp")
  (func (export "on_update") (param f32)
    (call $set (i32.const 0) (i32.const 1)
      (f32.convert_i32_s (call $sphere (f32.const 0) (f32.const 0) (f32.const 0) (f32.const 2) (i32.const 400) (i32.const 4))))
    (call $set (i32.const 8) (i32.const 5) (f32.convert_i32_s (i32.load (i32.const 400))))
    (drop (call $value (i32.const 7) (i32.const 40) (i32.const 2) (i32.const 500)))
    (call $set (i32.const 16) (i32.const 2) (f32.load (i32.const 500)))
    (call $set (i32.const 24) (i32.const 4)
      (f32.convert_i32_s (call $value (i32.const 8) (i32.const 40) (i32.const 2) (i32.const 504))))))
"#;

#[test]
fn overlap_sphere_and_get_value_of_answer_through_the_host() {
    for kind in backends() {
        let mut rt = runtime(kind, ScriptConfig::default());
        let id = rt.instantiate(&wasm(FIELD_QUERIES)).unwrap();
        rt.set_world_queries(id, Box::new(Field));
        rt.update(id, 0.016).unwrap();
        assert_eq!(rt.read_output(id, "n"), Some(2.0), "{kind:?}");
        assert_eq!(rt.read_output(id, "first"), Some(1.0), "{kind:?}");
        assert_eq!(rt.read_output(id, "hp"), Some(80.0), "{kind:?}");
        assert_eq!(rt.read_output(id, "none"), Some(0.0), "{kind:?}: no such entity");
    }
}

/// Asks for a path from (1, 0, 0) to (1, 0, 4) into the buffer at 700 (room for 8 points) and
/// from (-1, 0, 0), which has no path, into the same buffer.
const PATH_QUERY: &str = r#"
(module
  (import "env" "nav_path" (func $path (param i32 i32 i32 i32) (result i32)))
  (import "env" "set_f32" (func $set (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "count")
  (data (i32.const 16) "last_z")
  (data (i32.const 32) "none")
  (func (export "on_update") (param f32)
    (f32.store (i32.const 600) (f32.const 1))
    (f32.store (i32.const 604) (f32.const 0))
    (f32.store (i32.const 608) (f32.const 0))
    (f32.store (i32.const 612) (f32.const 1))
    (f32.store (i32.const 616) (f32.const 0))
    (f32.store (i32.const 620) (f32.const 4))
    (f32.store (i32.const 624) (f32.const -1))
    (f32.store (i32.const 628) (f32.const 0))
    (f32.store (i32.const 632) (f32.const 0))
    (f32.store (i32.const 636) (f32.const 0))
    (f32.store (i32.const 640) (f32.const 0))
    (f32.store (i32.const 644) (f32.const 0))
    (call $set (i32.const 0) (i32.const 5)
      (f32.convert_i32_s (call $path (i32.const 600) (i32.const 612) (i32.const 700) (i32.const 8))))
    (call $set (i32.const 16) (i32.const 6) (f32.load (i32.const 720)))
    (call $set (i32.const 32) (i32.const 4)
      (f32.convert_i32_s (call $path (i32.const 624) (i32.const 612) (i32.const 800) (i32.const 8))))))
"#;

#[test]
fn nav_path_returns_the_points_or_zero_without_a_path() {
    for kind in backends() {
        let mut rt = runtime(kind, ScriptConfig::default());
        let id = rt.instantiate(&wasm(PATH_QUERY)).unwrap();
        rt.set_world_queries(id, Box::new(Field));
        rt.update(id, 0.016).unwrap();
        assert_eq!(rt.read_output(id, "count"), Some(2.0), "{kind:?}");
        assert_eq!(rt.read_output(id, "last_z"), Some(4.0), "{kind:?}");
        assert_eq!(rt.read_output(id, "none"), Some(0.0), "{kind:?}");
    }
}

#[test]
fn the_value_and_nav_permissions_gate_their_imports() {
    for kind in backends() {
        let without_nav = ScriptConfig {
            api: ApiPermissions {
                nav: false,
                ..ApiPermissions::all()
            },
            ..ScriptConfig::default()
        };
        let mut rt = runtime(kind, without_nav);
        assert!(
            matches!(rt.instantiate(&wasm(PATH_QUERY)), Err(ScriptError::Link(_))),
            "{kind:?}: nav_path needs `nav`"
        );

        let without_values = ScriptConfig {
            api: ApiPermissions {
                entity_values: false,
                ..ApiPermissions::all()
            },
            ..ScriptConfig::default()
        };
        let mut rt = runtime(kind, without_values);
        assert!(
            matches!(rt.instantiate(&wasm(FIELD_QUERIES)), Err(ScriptError::Link(_))),
            "{kind:?}: get_value_of needs `entity_values`"
        );
    }
}
