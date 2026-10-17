//! The `wasmtime` backend must honour the same contract as the default `wasmi` backend.

#![cfg(feature = "wasmtime")]
#![allow(clippy::unwrap_used)]

use redox_script::{BackendKind, Permissions, ScriptConfig, ScriptError, ScriptRuntime};

fn runtime(config: ScriptConfig) -> ScriptRuntime {
    ScriptRuntime::with_backend(config, BackendKind::Wasmtime).unwrap()
}

fn wasm(text: &str) -> Vec<u8> {
    wat::parse_str(text).unwrap()
}

const MODULE: &str = r#"
(module
  (import "env" "log" (func $log (param i32 i32)))
  (import "env" "time_seconds" (func $time (result f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "hello")
  (global $t (export "t_seen") (mut f32) (f32.const 0))
  (func (export "on_update") (param f32)
    (global.set $t (call $time))
    (call $log (i32.const 0) (i32.const 5))
    (call $log (i32.const -1) (i32.const 100)))
  (func (export "try_grow") (result i32) (memory.grow (i32.const 1000))))
"#;

#[test]
fn logs_clock_and_bad_log_ranges() {
    let mut rt = runtime(ScriptConfig::default());
    let id = rt.instantiate(&wasm(MODULE)).unwrap();
    rt.update(id, 0.5).unwrap();
    rt.update(id, 0.25).unwrap();
    assert_eq!(rt.read_global_f32(id, "t_seen"), Some(0.75));
    assert_eq!(rt.take_logs(id), vec!["hello", "hello"]);
}

#[test]
fn memory_limit_and_grow_failure() {
    let small = ScriptConfig {
        memory_limit_bytes: 16 * 65536,
        ..ScriptConfig::default()
    };
    let mut rt = runtime(small);
    let id = rt.instantiate(&wasm(MODULE)).unwrap();
    assert_eq!(rt.call_i32(id, "try_grow").unwrap(), -1);

    let big = wasm(r#"(module (memory 100) (func (export "on_update") (param f32)))"#);
    assert!(rt.instantiate(&big).is_err());
}

#[test]
fn fuel_and_traps_disable_only_the_instance() {
    let mut rt = runtime(ScriptConfig {
        fuel_per_call: 100_000,
        ..ScriptConfig::default()
    });
    let spin = wasm(r#"(module (func (export "on_update") (param f32) (loop $l (br $l))))"#);
    let trap = wasm(r#"(module (func (export "on_update") (param f32) unreachable))"#);
    let spinner = rt.instantiate(&spin).unwrap();
    let trapper = rt.instantiate(&trap).unwrap();
    let healthy = rt.instantiate(&wasm(MODULE)).unwrap();

    assert!(matches!(
        rt.update(spinner, 0.1),
        Err(ScriptError::OutOfFuel)
    ));
    assert!(matches!(rt.update(trapper, 0.1), Err(ScriptError::Trap(_))));
    assert!(matches!(
        rt.update(spinner, 0.1),
        Err(ScriptError::Disabled)
    ));
    assert!(rt.is_disabled(trapper));
    rt.update(healthy, 0.1).unwrap();
}

#[test]
fn permissions_are_enforced() {
    let mut rt = runtime(ScriptConfig {
        permissions: Permissions {
            log: true,
            time: false,
        },
        ..ScriptConfig::default()
    });
    assert!(matches!(
        rt.instantiate(&wasm(MODULE)),
        Err(ScriptError::Link(_))
    ));
    assert!(matches!(
        rt.instantiate(b"nope"),
        Err(ScriptError::Compile(_))
    ));
}
