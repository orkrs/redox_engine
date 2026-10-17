//! Specification (tests first) for the WASM script runtime prototype (plan item B6).
//!
//! API to implement in `redox_script` (re-exported from the crate root):
//!
//! ```text
//! pub struct Permissions { pub log: bool, pub time: bool }
//! impl Permissions { pub const fn all() -> Self; pub const fn none() -> Self }
//! pub struct ScriptConfig { pub memory_limit_bytes: usize, pub fuel_per_call: u64, pub permissions: Permissions }
//! impl Default for ScriptConfig   // 16 MiB, 1_000_000 fuel, Permissions::all()
//! #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct InstanceId(pub u32);
//! #[derive(Debug)] pub enum ScriptError { Compile(String), Link(String), MissingExport(String),
//!                                         Trap(String), OutOfFuel, Disabled }   // + Display (thiserror)
//! pub struct ScriptRuntime;
//! impl ScriptRuntime {
//!     pub fn new(config: ScriptConfig) -> Self;
//!     /// Compile + link + instantiate. The module must export `on_update(f32)`.
//!     pub fn instantiate(&mut self, wasm: &[u8]) -> Result<InstanceId, ScriptError>;
//!     /// Adds `dt` to the instance's clock, then calls `on_update(dt)` with a fresh fuel budget.
//!     pub fn update(&mut self, id: InstanceId, dt: f32) -> Result<(), ScriptError>;
//!     pub fn take_logs(&mut self, id: InstanceId) -> Vec<String>;
//!     pub fn read_global_f32(&self, id: InstanceId, name: &str) -> Option<f32>;
//!     /// Calls an exported function `() -> i32` (fresh fuel budget).
//!     pub fn call_i32(&mut self, id: InstanceId, name: &str) -> Result<i32, ScriptError>;
//!     pub fn is_disabled(&self, id: InstanceId) -> bool;
//! }
//! ```
//!
//! Host functions imported from module `"env"` (only when permitted; importing a function that is not
//! permitted or does not exist is a `Link` error at instantiate):
//!   * `log(ptr: i32, len: i32)`   — permission `log`; reads UTF-8 from the module's exported `memory`.
//!   * `time_seconds() -> f32`     — permission `time`; the instance's accumulated game time.
//!
//! Safety rules: a trap or running out of fuel disables the instance (later calls return
//! `ScriptError::Disabled`); other instances keep working. Memory growth beyond `memory_limit_bytes`
//! fails the way wasm expects (`memory.grow` returns -1); a module whose *initial* memory already
//! exceeds the limit fails to instantiate.
//!
//! Backend: `wasmi` for now. Keep the engine behind a small private trait (`ScriptBackend`) so that
//! `wasmtime` can be added later (see the task text).

#![allow(clippy::unwrap_used)]

use redox_script::{InstanceId, Permissions, ScriptConfig, ScriptError, ScriptRuntime};

fn wasm(text: &str) -> Vec<u8> {
    wat::parse_str(text).expect("valid wat")
}

fn runtime() -> ScriptRuntime {
    ScriptRuntime::new(ScriptConfig::default())
}

const LOGGER: &str = r#"
(module
  (import "env" "log" (func $log (param i32 i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "hello from wasm")
  (func (export "on_update") (param f32)
    (call $log (i32.const 0) (i32.const 15))))
"#;

const CLOCK: &str = r#"
(module
  (import "env" "time_seconds" (func $time (result f32)))
  (global $dt (export "dt_seen") (mut f32) (f32.const 0))
  (global $t (export "t_seen") (mut f32) (f32.const 0))
  (func (export "on_update") (param $d f32)
    (global.set $dt (local.get $d))
    (global.set $t (call $time))))
"#;

const SPINNER: &str = r#"
(module
  (func (export "on_update") (param f32)
    (loop $forever (br $forever))))
"#;

const TRAPPER: &str = r#"
(module
  (func (export "on_update") (param f32) unreachable))
"#;

const GROWER: &str = r#"
(module
  (memory 1)
  (func (export "on_update") (param f32))
  (func (export "try_grow") (result i32) (memory.grow (i32.const 1000))))
"#;

#[test]
fn a_script_can_log_through_the_host() {
    let mut rt = runtime();
    let id = rt.instantiate(&wasm(LOGGER)).unwrap();
    rt.update(id, 0.016).unwrap();
    rt.update(id, 0.016).unwrap();
    assert_eq!(rt.take_logs(id), vec!["hello from wasm", "hello from wasm"]);
    assert!(rt.take_logs(id).is_empty(), "logs are taken once");
}

#[test]
fn the_script_sees_dt_and_an_accumulating_clock() {
    let mut rt = runtime();
    let id = rt.instantiate(&wasm(CLOCK)).unwrap();
    rt.update(id, 0.5).unwrap();
    rt.update(id, 0.25).unwrap();
    assert_eq!(rt.read_global_f32(id, "dt_seen"), Some(0.25));
    assert_eq!(rt.read_global_f32(id, "t_seen"), Some(0.75));
    assert_eq!(rt.read_global_f32(id, "no_such_global"), None);
}

#[test]
fn instances_have_independent_state() {
    let mut rt = runtime();
    let a = rt.instantiate(&wasm(CLOCK)).unwrap();
    let b = rt.instantiate(&wasm(CLOCK)).unwrap();
    assert_ne!(a, b);
    rt.update(a, 1.0).unwrap();
    assert_eq!(rt.read_global_f32(a, "t_seen"), Some(1.0));
    assert_eq!(rt.read_global_f32(b, "t_seen"), Some(0.0));
}

#[test]
fn an_endless_loop_runs_out_of_fuel_and_only_that_instance_is_disabled() {
    let mut rt = ScriptRuntime::new(ScriptConfig {
        fuel_per_call: 100_000,
        ..ScriptConfig::default()
    });
    let spinner = rt.instantiate(&wasm(SPINNER)).unwrap();
    let healthy = rt.instantiate(&wasm(CLOCK)).unwrap();

    assert!(matches!(rt.update(spinner, 0.016), Err(ScriptError::OutOfFuel)));
    assert!(rt.is_disabled(spinner));
    assert!(matches!(rt.update(spinner, 0.016), Err(ScriptError::Disabled)));

    rt.update(healthy, 0.016).unwrap();
    assert!(!rt.is_disabled(healthy));
}

#[test]
fn a_trap_disables_the_instance_and_reports_it() {
    let mut rt = runtime();
    let id = rt.instantiate(&wasm(TRAPPER)).unwrap();
    assert!(matches!(rt.update(id, 0.016), Err(ScriptError::Trap(_))));
    assert!(rt.is_disabled(id));
    assert!(matches!(rt.update(id, 0.016), Err(ScriptError::Disabled)));
}

#[test]
fn memory_growth_is_capped_by_the_limit() {
    // 16 pages = 1 MiB: growing by 1000 pages must fail with -1.
    let mut small = ScriptRuntime::new(ScriptConfig {
        memory_limit_bytes: 16 * 65536,
        ..ScriptConfig::default()
    });
    let id = small.instantiate(&wasm(GROWER)).unwrap();
    assert_eq!(small.call_i32(id, "try_grow").unwrap(), -1);

    // 100 MiB allows it: memory.grow returns the previous size in pages (1).
    let mut large = ScriptRuntime::new(ScriptConfig {
        memory_limit_bytes: 100 * 1024 * 1024,
        ..ScriptConfig::default()
    });
    let id = large.instantiate(&wasm(GROWER)).unwrap();
    assert_eq!(large.call_i32(id, "try_grow").unwrap(), 1);
}

#[test]
fn a_module_that_starts_above_the_memory_limit_is_refused() {
    let big = wasm(r#"(module (memory 100) (func (export "on_update") (param f32)))"#);
    let mut rt = ScriptRuntime::new(ScriptConfig {
        memory_limit_bytes: 16 * 65536,
        ..ScriptConfig::default()
    });
    assert!(rt.instantiate(&big).is_err());
}

#[test]
fn imports_need_permission() {
    let mut rt = ScriptRuntime::new(ScriptConfig {
        permissions: Permissions { log: true, time: false },
        ..ScriptConfig::default()
    });
    assert!(matches!(rt.instantiate(&wasm(CLOCK)), Err(ScriptError::Link(_))), "time is not permitted");
    assert!(rt.instantiate(&wasm(LOGGER)).is_ok(), "log is permitted");

    let mut none = ScriptRuntime::new(ScriptConfig {
        permissions: Permissions::none(),
        ..ScriptConfig::default()
    });
    assert!(matches!(none.instantiate(&wasm(LOGGER)), Err(ScriptError::Link(_))));
    // A module that imports nothing needs no permission.
    assert!(none.instantiate(&wasm(TRAPPER)).is_ok());
}

#[test]
fn unknown_imports_are_link_errors() {
    let module = wasm(
        r#"(module (import "env" "does_not_exist" (func))
             (func (export "on_update") (param f32)))"#,
    );
    assert!(matches!(runtime().instantiate(&module), Err(ScriptError::Link(_))));
}

#[test]
fn bad_modules_are_rejected_with_the_right_error() {
    let mut rt = runtime();
    assert!(matches!(rt.instantiate(b"this is not wasm"), Err(ScriptError::Compile(_))));
    let no_update = wasm(r#"(module (func (export "something_else")))"#);
    match rt.instantiate(&no_update) {
        Err(ScriptError::MissingExport(name)) => assert_eq!(name, "on_update"),
        other => panic!("expected MissingExport, got {other:?}"),
    }
}

#[test]
fn calling_an_unknown_export_is_an_error_not_a_panic() {
    let mut rt = runtime();
    let id = rt.instantiate(&wasm(GROWER)).unwrap();
    assert!(matches!(rt.call_i32(id, "nope"), Err(ScriptError::MissingExport(_))));
    assert!(!rt.is_disabled(id), "a bad call does not disable the instance");
}

#[test]
fn unknown_instance_ids_do_not_panic() {
    let mut rt = runtime();
    let ghost = InstanceId(999);
    assert!(rt.update(ghost, 0.016).is_err());
    assert!(rt.take_logs(ghost).is_empty());
    assert_eq!(rt.read_global_f32(ghost, "x"), None);
}

#[test]
fn defaults_are_sane() {
    let c = ScriptConfig::default();
    assert_eq!(c.memory_limit_bytes, 16 * 1024 * 1024);
    assert_eq!(c.fuel_per_call, 1_000_000);
    assert!(c.permissions.log && c.permissions.time);
    assert!(!Permissions::none().log && !Permissions::none().time);
}
