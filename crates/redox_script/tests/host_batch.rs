//! Specification (tests first) for batched world queries (plan item B7c, first part).
//!
//! New host function (module `"env"`), permission `api.world` (same as `raycast`):
//!
//! ```text
//! raycast_many(rays_ptr: i32, count: i32, out_ptr: i32) -> i32
//! ```
//!
//! `rays_ptr` points at `count` records of 7 little-endian `f32` each (`ox, oy, oz, dx, dy, dz,
//! max_distance`, 28 bytes, same meaning as `raycast`); `out_ptr` receives `count` `f32` answers (the hit
//! distance or -1.0). Every ray goes through exactly the same checks and the same per-call allowance (64
//! host queries per call into the script) as a single `raycast`: rays beyond the allowance, malformed
//! rays and misses all answer -1.0. Returns `count` when the batch was processed.
//!
//! Robustness (never a trap, the instance stays enabled): `count <= 0` or `count > 1024` -> returns 0
//! and writes nothing; a record range or an output range outside the guest memory -> returns 0 and
//! writes nothing; no world installed -> processes the batch, every answer -1.0.
//!
//! Where it goes: `host.rs` gets `raycast_many(&mut self, rays: &[u8], out: &mut Vec<f32>)` style shared
//! logic (parse records, call `self.raycast` for each); both backends read the guest memory export,
//! call it, write the answers back as little-endian `f32`. Implement on `wasmi` and `wasmtime`. Add the
//! import next to `raycast` in both `build_linker` functions.

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
    #[cfg_attr(not(feature = "wasmtime"), allow(unused_mut))]
    let mut kinds = vec![BackendKind::Wasmi];
    #[cfg(feature = "wasmtime")]
    kinds.push(BackendKind::Wasmtime);
    kinds
}

fn runtime(kind: BackendKind) -> ScriptRuntime {
    ScriptRuntime::with_backend(ScriptConfig::default(), kind).unwrap()
}

/// A flat ground at y = 0 that counts the queries it receives.
#[derive(Default)]
struct Ground {
    calls: Rc<RefCell<u32>>,
}

impl WorldQueries for Ground {
    fn raycast(&mut self, origin: [f32; 3], direction: [f32; 3], max_distance: f32) -> Option<f32> {
        *self.calls.borrow_mut() += 1;
        if direction[1] >= 0.0 || origin[1] <= 0.0 {
            return None;
        }
        let distance = origin[1] / -direction[1];
        (distance <= max_distance).then_some(distance)
    }
}

/// Three rays written into memory at 64 (28 bytes each), answers at 512, results exported as outputs
/// `r0`, `r1`, `r2` and the return value as `n`.
const THREE_RAYS: &str = r#"
(module
  (import "env" "raycast_many" (func $many (param i32 i32 i32) (result i32)))
  (import "env" "set_f32" (func $set (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "r0r1r2n")
  (func $ray (param $at i32) (param $ox f32) (param $oy f32) (param $oz f32)
             (param $dx f32) (param $dy f32) (param $dz f32) (param $max f32)
    (f32.store offset=0 (local.get $at) (local.get $ox))
    (f32.store offset=4 (local.get $at) (local.get $oy))
    (f32.store offset=8 (local.get $at) (local.get $oz))
    (f32.store offset=12 (local.get $at) (local.get $dx))
    (f32.store offset=16 (local.get $at) (local.get $dy))
    (f32.store offset=20 (local.get $at) (local.get $dz))
    (f32.store offset=24 (local.get $at) (local.get $max)))
  (func (export "on_update") (param f32)
    (call $ray (i32.const 64)  (f32.const 0) (f32.const 2) (f32.const 0) (f32.const 0) (f32.const -1) (f32.const 0) (f32.const 10))
    (call $ray (i32.const 92)  (f32.const 0) (f32.const 5) (f32.const 0) (f32.const 0) (f32.const -1) (f32.const 0) (f32.const 3))
    (call $ray (i32.const 120) (f32.const 1) (f32.const 4) (f32.const 0) (f32.const 0) (f32.const -1) (f32.const 0) (f32.const 10))
    (call $set (i32.const 6) (i32.const 1)
      (f32.convert_i32_s (call $many (i32.const 64) (i32.const 3) (i32.const 512))))
    (call $set (i32.const 0) (i32.const 2) (f32.load (i32.const 512)))
    (call $set (i32.const 2) (i32.const 2) (f32.load (i32.const 516)))
    (call $set (i32.const 4) (i32.const 2) (f32.load (i32.const 520)))))
"#;

#[test]
fn a_batch_answers_like_single_rays() {
    for kind in backends() {
        let mut rt = runtime(kind);
        let id = rt.instantiate(&wasm(THREE_RAYS)).unwrap();
        let ground = Ground::default();
        let calls = Rc::clone(&ground.calls);
        rt.set_world_queries(id, Box::new(ground));
        rt.update(id, 0.016).unwrap();
        assert_eq!(rt.read_output(id, "n"), Some(3.0), "{kind:?}");
        assert_eq!(rt.read_output(id, "r0"), Some(2.0));
        assert_eq!(rt.read_output(id, "r1"), Some(-1.0), "too far for max 3");
        assert_eq!(rt.read_output(id, "r2"), Some(4.0));
        assert_eq!(*calls.borrow(), 3);
    }
}

#[test]
fn without_a_world_every_answer_is_minus_one() {
    for kind in backends() {
        let mut rt = runtime(kind);
        let id = rt.instantiate(&wasm(THREE_RAYS)).unwrap();
        rt.update(id, 0.016).unwrap();
        assert_eq!(rt.read_output(id, "n"), Some(3.0), "{kind:?}");
        assert_eq!(rt.read_output(id, "r0"), Some(-1.0));
        assert_eq!(rt.read_output(id, "r2"), Some(-1.0));
    }
}

#[test]
fn the_world_permission_is_required() {
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
            rt.instantiate(&wasm(THREE_RAYS)),
            Err(ScriptError::Link(_))
        ));
    }
}

/// Parameterised batch: rays at 64 (all the same downward ray written by a loop), `count` rays, the output
/// at 4096; stores the return value as `n` and the answers 0, 63, 64 and 69 as `a0`, `a63`, `a64`, `a69`.
fn batch_module(count: i32, rays_ptr: i32, out_ptr: i32) -> String {
    format!(
        r#"
(module
  (import "env" "raycast_many" (func $many (param i32 i32 i32) (result i32)))
  (import "env" "set_f32" (func $set (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "na0a63a64a69")
  (func (export "on_update") (param f32)
    (local $i i32) (local $at i32)
    (loop $fill
      (local.set $at (i32.add (i32.const 64) (i32.mul (local.get $i) (i32.const 28))))
      (f32.store offset=0 (local.get $at) (f32.const 0))
      (f32.store offset=4 (local.get $at) (f32.const 2))
      (f32.store offset=8 (local.get $at) (f32.const 0))
      (f32.store offset=12 (local.get $at) (f32.const 0))
      (f32.store offset=16 (local.get $at) (f32.const -1))
      (f32.store offset=20 (local.get $at) (f32.const 0))
      (f32.store offset=24 (local.get $at) (f32.const 10))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $fill (i32.lt_u (local.get $i) (i32.const 70))))
    (call $set (i32.const 0) (i32.const 1)
      (f32.convert_i32_s (call $many (i32.const {rays_ptr}) (i32.const {count}) (i32.const {out_ptr}))))
    (call $set (i32.const 1) (i32.const 2) (f32.load (i32.const 4096)))
    (call $set (i32.const 3) (i32.const 3) (f32.load (i32.const 4348)))
    (call $set (i32.const 6) (i32.const 3) (f32.load (i32.const 4352)))
    (call $set (i32.const 9) (i32.const 3) (f32.load (i32.const 4372)))))
"#
    )
}

#[test]
fn rays_beyond_the_per_call_allowance_answer_minus_one() {
    for kind in backends() {
        let mut rt = runtime(kind);
        let id = rt.instantiate(&wasm(&batch_module(70, 64, 4096))).unwrap();
        let ground = Ground::default();
        let calls = Rc::clone(&ground.calls);
        rt.set_world_queries(id, Box::new(ground));
        rt.update(id, 0.016).unwrap();
        assert_eq!(rt.read_output(id, "n"), Some(70.0), "{kind:?}");
        assert_eq!(rt.read_output(id, "a0"), Some(2.0));
        assert_eq!(
            rt.read_output(id, "a63"),
            Some(2.0),
            "the 64th ray still counts"
        );
        assert_eq!(rt.read_output(id, "a64"), Some(-1.0), "the 65th does not");
        assert_eq!(rt.read_output(id, "a69"), Some(-1.0));
        assert_eq!(*calls.borrow(), 64);
    }
}

#[test]
fn bad_counts_and_ranges_return_zero_and_write_nothing() {
    for (count, rays_ptr, out_ptr) in [
        (0, 64, 4096),
        (-5, 64, 4096),
        (1025, 64, 4096),
        (3, 65_530, 4096), // the record range leaves the memory
        (3, 64, 65_530),   // the output range leaves the memory
        (3, -4, 4096),     // negative pointer
        (3, 64, i32::MAX), // far outside
    ] {
        for kind in backends() {
            let mut rt = runtime(kind);
            let id = rt
                .instantiate(&wasm(&batch_module(count, rays_ptr, out_ptr)))
                .unwrap();
            rt.set_world_queries(id, Box::new(Ground::default()));
            rt.update(id, 0.016).unwrap();
            assert_eq!(
                rt.read_output(id, "n"),
                Some(0.0),
                "{kind:?} count {count} rays {rays_ptr} out {out_ptr}"
            );
            assert_eq!(rt.read_output(id, "a0"), Some(0.0), "nothing was written");
            assert!(!rt.is_disabled(id));
        }
    }
}
