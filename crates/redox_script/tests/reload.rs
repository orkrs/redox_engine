//! Specification (tests first) for hot reload of a script module (SCR-01, `ScriptRuntime::reload`).
//!
//! `reload` replaces the module of an existing instance id. Inputs set so far are kept; outputs,
//! queued events/commands, logs, globals and the clock start over. A failed reload leaves the old
//! module running. A trapped (disabled) instance is enabled again by a successful reload.

#![allow(clippy::unwrap_used)]

use redox_script::{BackendKind, InstanceId, ScriptConfig, ScriptError, ScriptRuntime};

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

/// Reads the input `in` and stores `out = in + 1`.
const V1: &str = r#"
(module
  (import "env" "get_f32" (func $g (param i32 i32) (result f32)))
  (import "env" "set_f32" (func $s (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "inout")
  (func (export "on_update") (param f32)
    (call $s (i32.const 2) (i32.const 3)
      (f32.add (call $g (i32.const 0) (i32.const 2)) (f32.const 1)))))
"#;

/// Same as `V1` but stores `out = in + 100`.
const V2: &str = r#"
(module
  (import "env" "get_f32" (func $g (param i32 i32) (result f32)))
  (import "env" "set_f32" (func $s (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "inout")
  (func (export "on_update") (param f32)
    (call $s (i32.const 2) (i32.const 3)
      (f32.add (call $g (i32.const 0) (i32.const 2)) (f32.const 100)))))
"#;

/// Compiles and links, but has no `on_update` export.
const NO_UPDATE: &str = r#"(module (memory (export "memory") 1))"#;

/// Traps on every update.
const TRAPS: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "on_update") (param f32) unreachable))
"#;

/// Queues one event `done` on every update.
const EMITS: &str = r#"
(module
  (import "env" "emit_event" (func $e (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "done")
  (func (export "on_update") (param f32)
    (call $e (i32.const 0) (i32.const 4) (f32.const 1))))
"#;

#[test]
fn reload_swaps_the_logic_and_keeps_the_inputs() {
    for kind in backends() {
        let mut rt = runtime(kind);
        let id = rt.instantiate(&wasm(V1)).unwrap();
        assert!(rt.set_input(id, "in", 5.0));
        rt.update(id, 0.016).unwrap();
        assert_eq!(rt.read_output(id, "out"), Some(6.0), "{kind:?}");

        rt.reload(id, &wasm(V2)).unwrap();
        assert_eq!(
            rt.read_output(id, "out"),
            None,
            "outputs start over ({kind:?})"
        );
        rt.update(id, 0.016).unwrap();
        assert_eq!(
            rt.read_output(id, "out"),
            Some(105.0),
            "input kept ({kind:?})"
        );
    }
}

#[test]
fn a_broken_module_leaves_the_old_one_running() {
    for kind in backends() {
        let mut rt = runtime(kind);
        let id = rt.instantiate(&wasm(V1)).unwrap();
        rt.set_input(id, "in", 5.0);
        rt.update(id, 0.016).unwrap();

        let err = rt.reload(id, &[1, 2, 3]).unwrap_err();
        assert!(matches!(err, ScriptError::Compile(_)), "{kind:?}: {err:?}");

        rt.update(id, 0.016).unwrap();
        assert_eq!(
            rt.read_output(id, "out"),
            Some(6.0),
            "old module kept ({kind:?})"
        );
    }
}

#[test]
fn a_module_without_on_update_is_rejected_and_the_old_one_stays() {
    for kind in backends() {
        let mut rt = runtime(kind);
        let id = rt.instantiate(&wasm(V1)).unwrap();
        rt.set_input(id, "in", 5.0);

        let err = rt.reload(id, &wasm(NO_UPDATE)).unwrap_err();
        assert!(
            matches!(err, ScriptError::MissingExport(_)),
            "{kind:?}: {err:?}"
        );

        rt.update(id, 0.016).unwrap();
        assert_eq!(
            rt.read_output(id, "out"),
            Some(6.0),
            "old module kept ({kind:?})"
        );
    }
}

#[test]
fn reload_revives_a_trapped_instance() {
    for kind in backends() {
        let mut rt = runtime(kind);
        let id = rt.instantiate(&wasm(TRAPS)).unwrap();
        let err = rt.update(id, 0.016).unwrap_err();
        assert!(matches!(err, ScriptError::Trap(_)), "{kind:?}: {err:?}");
        assert!(rt.is_disabled(id), "{kind:?}");

        rt.reload(id, &wasm(V1)).unwrap();
        assert!(!rt.is_disabled(id), "{kind:?}");
        rt.update(id, 0.016).unwrap();
        assert_eq!(
            rt.read_output(id, "out"),
            Some(1.0),
            "input defaults to 0 ({kind:?})"
        );
    }
}

#[test]
fn reload_of_an_unknown_instance_is_an_error() {
    for kind in backends() {
        let mut rt = runtime(kind);
        let err = rt.reload(InstanceId(9), &wasm(V1)).unwrap_err();
        assert!(matches!(err, ScriptError::Disabled), "{kind:?}: {err:?}");
    }
}

#[test]
fn reload_drops_queued_events() {
    for kind in backends() {
        let mut rt = runtime(kind);
        let id = rt.instantiate(&wasm(EMITS)).unwrap();
        rt.update(id, 0.016).unwrap();

        rt.reload(id, &wasm(V1)).unwrap();
        assert!(rt.take_events(id).is_empty(), "{kind:?}");
    }
}
