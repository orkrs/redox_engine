//! Specification (tests first) for the script host API v1b: queries into the game world and a
//! gameplay module written in WebAssembly (plan item B7, second part).
//!
//! API added to `redox_script` (re-exported from the crate root):
//!
//! ```text
//! /// The game world as scripts may see it. Implemented by the host (for example on top of the
//! /// physics context) and owned by the runtime instance it is installed on.
//! pub trait WorldQueries {
//!     /// Distance to the first solid surface along the ray, or `None` when nothing is hit within
//!     /// `max_distance`. `direction` is a unit vector (the runtime normalises it first).
//!     fn raycast(&mut self, origin: [f32; 3], direction: [f32; 3], max_distance: f32) -> Option<f32>;
//! }
//! impl ScriptRuntime {
//!     /// Installs the world for one instance. False for an unknown instance.
//!     pub fn set_world_queries(&mut self, id: InstanceId, world: Box<dyn WorldQueries>) -> bool;
//! }
//! // ApiPermissions gets a third switch `world` (all(): true, none(): false).
//! ```
//!
//! Host function (module `"env"`), permission `api.world`:
//!   * `raycast(ox, oy, oz, dx, dy, dz, max_distance: f32) -> f32`: the hit distance, or -1.0 for no hit.
//!
//! Robustness: no world installed -> -1.0. Any non finite argument, a zero direction or
//! `max_distance <= 0` -> -1.0 without calling the host. `max_distance` is capped at 1000 m. The
//! direction is normalised before the host sees it. At most 64 queries per call into the script
//! (`on_update` or `call_i32`); further queries in the same call return -1.0 without calling the host.
//! A fresh call gets a fresh allowance. Never a trap.

#![allow(clippy::unwrap_used)]

use std::cell::RefCell;
use std::rc::Rc;

use redox_script::{
    ApiPermissions, BackendKind, ScriptConfig, ScriptError, ScriptRuntime, WorldQueries,
};

fn wasm(text: &str) -> Vec<u8> {
    wat::parse_str(text).expect("valid wat")
}

fn backends() -> Vec<BackendKind> {
    let mut kinds = vec![BackendKind::Wasmi];
    #[cfg(feature = "wasmtime")]
    kinds.push(BackendKind::Wasmtime);
    kinds
}

fn runtime(kind: BackendKind) -> ScriptRuntime {
    ScriptRuntime::with_backend(ScriptConfig::default(), kind).unwrap()
}

/// A flat ground at y = 0 that logs every query it receives.
#[derive(Default)]
struct Ground {
    calls: Rc<RefCell<Vec<([f32; 3], [f32; 3], f32)>>>,
}

impl WorldQueries for Ground {
    fn raycast(&mut self, origin: [f32; 3], direction: [f32; 3], max_distance: f32) -> Option<f32> {
        self.calls
            .borrow_mut()
            .push((origin, direction, max_distance));
        if direction[1] >= 0.0 || origin[1] <= 0.0 {
            return None;
        }
        let distance = origin[1] / -direction[1];
        (distance <= max_distance).then_some(distance)
    }
}

/// Casts a ray from (0, 2, 0) straight down, max 10 m, and stores the answer as "hit".
const DOWN_RAY: &str = r#"
(module
  (import "env" "raycast" (func $ray (param f32 f32 f32 f32 f32 f32 f32) (result f32)))
  (import "env" "set_f32" (func $set (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "hit")
  (func (export "on_update") (param f32)
    (call $set (i32.const 0) (i32.const 3)
      (call $ray (f32.const 0) (f32.const 2) (f32.const 0)
                 (f32.const 0) (f32.const -1) (f32.const 0)
                 (f32.const 10)))))
"#;

#[test]
fn a_script_sees_the_distance_to_the_ground() {
    for kind in backends() {
        let mut rt = runtime(kind);
        let id = rt.instantiate(&wasm(DOWN_RAY)).unwrap();
        assert!(rt.set_world_queries(id, Box::new(Ground::default())));
        rt.update(id, 0.016).unwrap();
        assert_eq!(rt.read_output(id, "hit"), Some(2.0), "{kind:?}");
    }
}

#[test]
fn a_miss_is_minus_one() {
    for kind in backends() {
        let mut rt = runtime(kind);
        let id = rt.instantiate(&wasm(DOWN_RAY)).unwrap();
        // A world that never hits.
        struct Nothing;
        impl WorldQueries for Nothing {
            fn raycast(&mut self, _: [f32; 3], _: [f32; 3], _: f32) -> Option<f32> {
                None
            }
        }
        rt.set_world_queries(id, Box::new(Nothing));
        rt.update(id, 0.016).unwrap();
        assert_eq!(rt.read_output(id, "hit"), Some(-1.0), "{kind:?}");
    }
}

#[test]
fn without_a_world_every_query_misses_and_nothing_traps() {
    for kind in backends() {
        let mut rt = runtime(kind);
        let id = rt.instantiate(&wasm(DOWN_RAY)).unwrap();
        rt.update(id, 0.016).unwrap();
        assert_eq!(rt.read_output(id, "hit"), Some(-1.0), "{kind:?}");
        assert!(!rt.is_disabled(id));
    }
}

#[test]
fn the_world_permission_is_required_to_import_raycast() {
    for kind in backends() {
        let config = ScriptConfig {
            api: ApiPermissions {
                world: false,
                ..ApiPermissions::all()
            },
            ..ScriptConfig::default()
        };
        let mut rt = ScriptRuntime::with_backend(config, kind).unwrap();
        assert!(matches!(
            rt.instantiate(&wasm(DOWN_RAY)),
            Err(ScriptError::Link(_))
        ));
    }
    assert!(ApiPermissions::all().world);
    assert!(!ApiPermissions::none().world);
}

#[test]
fn an_unknown_instance_cannot_get_a_world() {
    let mut rt = runtime(BackendKind::Wasmi);
    assert!(!rt.set_world_queries(redox_script::InstanceId(7), Box::new(Ground::default())));
}

/// Casts a sideways ray with a non-unit direction (3, 0, 4), max 5000 m.
const SIDE_RAY: &str = r#"
(module
  (import "env" "raycast" (func $ray (param f32 f32 f32 f32 f32 f32 f32) (result f32)))
  (memory (export "memory") 1)
  (func (export "on_update") (param f32)
    (drop (call $ray (f32.const 1) (f32.const 2) (f32.const 3)
                     (f32.const 3) (f32.const 0) (f32.const 4)
                     (f32.const 5000)))))
"#;

#[test]
fn the_host_receives_a_unit_direction_and_a_capped_distance() {
    for kind in backends() {
        let mut rt = runtime(kind);
        let id = rt.instantiate(&wasm(SIDE_RAY)).unwrap();
        let ground = Ground::default();
        let calls = Rc::clone(&ground.calls);
        rt.set_world_queries(id, Box::new(ground));
        rt.update(id, 0.016).unwrap();
        let calls = calls.borrow();
        assert_eq!(calls.len(), 1, "{kind:?}");
        let (origin, direction, max) = calls[0];
        assert_eq!(origin, [1.0, 2.0, 3.0]);
        assert!((direction[0] - 0.6).abs() < 1e-6 && (direction[2] - 0.8).abs() < 1e-6);
        assert_eq!(max, 1000.0);
    }
}

/// Four malformed rays: NaN origin, zero direction, negative distance, infinite distance.
const BAD_RAYS: &str = r#"
(module
  (import "env" "raycast" (func $ray (param f32 f32 f32 f32 f32 f32 f32) (result f32)))
  (import "env" "set_f32" (func $set (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "sum")
  (func (export "on_update") (param f32)
    (call $set (i32.const 0) (i32.const 3)
      (f32.add
        (f32.add
          (call $ray (f32.const nan) (f32.const 0) (f32.const 0) (f32.const 0) (f32.const -1) (f32.const 0) (f32.const 10))
          (call $ray (f32.const 0) (f32.const 1) (f32.const 0) (f32.const 0) (f32.const 0) (f32.const 0) (f32.const 10)))
        (f32.add
          (call $ray (f32.const 0) (f32.const 1) (f32.const 0) (f32.const 0) (f32.const -1) (f32.const 0) (f32.const -4))
          (call $ray (f32.const 0) (f32.const 1) (f32.const 0) (f32.const 0) (f32.const -1) (f32.const 0) (f32.const inf)))))))
"#;

#[test]
fn malformed_rays_return_minus_one_without_calling_the_host() {
    for kind in backends() {
        let mut rt = runtime(kind);
        let id = rt.instantiate(&wasm(BAD_RAYS)).unwrap();
        let ground = Ground::default();
        let calls = Rc::clone(&ground.calls);
        rt.set_world_queries(id, Box::new(ground));
        rt.update(id, 0.016).unwrap();
        assert_eq!(rt.read_output(id, "sum"), Some(-4.0), "{kind:?}");
        assert!(calls.borrow().is_empty(), "the host was never asked");
        assert!(!rt.is_disabled(id));
    }
}

/// 100 rays in one call; the sum of the answers is stored.
const MANY_RAYS: &str = r#"
(module
  (import "env" "raycast" (func $ray (param f32 f32 f32 f32 f32 f32 f32) (result f32)))
  (import "env" "set_f32" (func $set (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "sum")
  (func (export "on_update") (param f32)
    (local $i i32) (local $sum f32)
    (loop $again
      (local.set $sum (f32.add (local.get $sum)
        (call $ray (f32.const 0) (f32.const 2) (f32.const 0) (f32.const 0) (f32.const -1) (f32.const 0) (f32.const 10))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $again (i32.lt_u (local.get $i) (i32.const 100))))
    (call $set (i32.const 0) (i32.const 3) (local.get $sum))))
"#;

#[test]
fn only_64_queries_are_answered_per_call_and_the_allowance_renews() {
    for kind in backends() {
        let mut rt = runtime(kind);
        let id = rt.instantiate(&wasm(MANY_RAYS)).unwrap();
        let ground = Ground::default();
        let calls = Rc::clone(&ground.calls);
        rt.set_world_queries(id, Box::new(ground));
        rt.update(id, 0.016).unwrap();
        // 64 hits at 2.0 and 36 refusals at -1.0.
        assert_eq!(
            rt.read_output(id, "sum"),
            Some(64.0 * 2.0 - 36.0),
            "{kind:?}"
        );
        assert_eq!(calls.borrow().len(), 64);
        rt.update(id, 0.016).unwrap();
        assert_eq!(calls.borrow().len(), 128, "a new call, a new allowance");
    }
}

/// A coin collector written as a game module: three coins, a pickup radius of 0.5 m, the player position
/// arrives as the inputs `player_x` / `player_z`, every pickup is the event `coin` with the coin index.
const COINS: &str = r#"
(module
  (import "env" "get_f32" (func $get (param i32 i32) (result f32)))
  (import "env" "emit_event" (func $emit (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "coin")
  (data (i32.const 8) "player_x")
  (data (i32.const 16) "player_z")
  (global $mask (mut i32) (i32.const 0))
  (global $count (export "collected") (mut f32) (f32.const 0))
  (func $check (param $i i32) (param $cx f32) (param $cz f32)
    (local $dx f32) (local $dz f32) (local $bit i32)
    (local.set $bit (i32.shl (i32.const 1) (local.get $i)))
    (if (i32.and (global.get $mask) (local.get $bit)) (then (return)))
    (local.set $dx (f32.sub (call $get (i32.const 8) (i32.const 8)) (local.get $cx)))
    (local.set $dz (f32.sub (call $get (i32.const 16) (i32.const 8)) (local.get $cz)))
    (if (f32.lt
          (f32.add (f32.mul (local.get $dx) (local.get $dx)) (f32.mul (local.get $dz) (local.get $dz)))
          (f32.const 0.25))
      (then
        (global.set $mask (i32.or (global.get $mask) (local.get $bit)))
        (global.set $count (f32.add (global.get $count) (f32.const 1)))
        (call $emit (i32.const 0) (i32.const 4) (f32.convert_i32_u (local.get $i))))))
  (func (export "on_update") (param f32)
    (call $check (i32.const 0) (f32.const 2) (f32.const 0))
    (call $check (i32.const 1) (f32.const 5) (f32.const 0))
    (call $check (i32.const 2) (f32.const 5) (f32.const 3))))
"#;

#[test]
fn coins_collected_by_a_wasm_module_are_reported_once_each() {
    for kind in backends() {
        let mut rt = runtime(kind);
        let id = rt.instantiate(&wasm(COINS)).unwrap();
        let walk = |rt: &mut ScriptRuntime, x: f32, z: f32| {
            rt.set_input(id, "player_x", x);
            rt.set_input(id, "player_z", z);
            rt.update(id, 0.016).unwrap();
            rt.take_events(id)
                .into_iter()
                .map(|e| {
                    assert_eq!(e.name, "coin");
                    e.value as u32
                })
                .collect::<Vec<_>>()
        };
        assert!(walk(&mut rt, 0.0, 0.0).is_empty(), "{kind:?}: nothing near");
        assert_eq!(walk(&mut rt, 2.1, 0.1), vec![0]);
        assert!(walk(&mut rt, 2.0, 0.0).is_empty(), "a coin is taken once");
        assert_eq!(walk(&mut rt, 5.0, 0.2), vec![1]);
        assert_eq!(walk(&mut rt, 5.0, 3.0), vec![2]);
        assert_eq!(rt.read_global_f32(id, "collected"), Some(3.0));
    }
}
