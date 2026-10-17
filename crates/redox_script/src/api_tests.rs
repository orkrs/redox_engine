//! Unit tests of the host API v1a (the full specification lives in `tests/host_api.rs`).

use crate::{ApiPermissions, ScriptConfig, ScriptError, ScriptEvent, ScriptRuntime};

const MODULE: &str = r#"
(module
  (import "env" "get_f32" (func $get (param i32 i32) (result f32)))
  (import "env" "set_f32" (func $set (param i32 i32 f32)))
  (import "env" "emit_event" (func $emit (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "in") (data (i32.const 8) "out") (data (i32.const 16) "\ff\ff")
  (global $bad (export "bad") (mut f32) (f32.const -1))
  (func (export "on_update") (param f32) (local $i i32)
    (call $set (i32.const 8) (i32.const 3) (f32.mul (call $get (i32.const 0) (i32.const 2)) (f32.const 2)))
    (call $emit (i32.const 8) (i32.const 3) (f32.const 7))
    (call $set (i32.const 70000) (i32.const 3) (f32.const 1))
    (call $set (i32.const 16) (i32.const 2) (f32.const 1))
    (call $emit (i32.const 0) (i32.const 100) (f32.const 1))
    (global.set $bad (call $get (i32.const 70000) (i32.const 3)))
    (loop $again
      (call $emit (i32.const 0) (i32.const 2) (f32.const 1))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $again (i32.lt_u (local.get $i) (i32.const 400))))))
"#;

fn check(mut rt: ScriptRuntime) {
    let wasm = wat::parse_str(MODULE).unwrap();
    let id = rt.instantiate(&wasm).unwrap();
    assert!(rt.set_input(id, "in", 4.0));
    rt.update(id, 0.1).unwrap();
    assert_eq!(rt.read_output(id, "out"), Some(8.0));
    assert_eq!(rt.read_output(id, "missing"), None);
    assert_eq!(rt.read_global_f32(id, "bad"), Some(0.0));
    let events = rt.take_events(id);
    assert_eq!(events.len(), 256);
    assert_eq!(
        events[0],
        ScriptEvent {
            name: "out".into(),
            value: 7.0
        }
    );
    assert!(rt.take_events(id).is_empty());
    assert!(!rt.is_disabled(id));
}

#[test]
fn wasmi_serves_the_api() {
    check(ScriptRuntime::new(ScriptConfig::default()));
}

#[cfg(feature = "wasmtime")]
#[test]
fn wasmtime_serves_the_api() {
    let rt = ScriptRuntime::with_backend(ScriptConfig::default(), crate::BackendKind::Wasmtime);
    check(rt.unwrap());
}

#[test]
fn unpermitted_api_imports_fail_to_link() {
    let config = ScriptConfig {
        api: ApiPermissions::none(),
        ..ScriptConfig::default()
    };
    let wasm = wat::parse_str(MODULE).unwrap();
    let result = ScriptRuntime::new(config).instantiate(&wasm);
    assert!(matches!(result, Err(ScriptError::Link(_))));
}
