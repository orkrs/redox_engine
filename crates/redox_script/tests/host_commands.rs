//! Specification (tests first) for script commands and bulk inputs (plan item B7c, second part).
//!
//! Events carry one number. Gameplay wants "do this with these three numbers" (spawn at x, y, z; move
//! to; play sound at volume). New API in `redox_script`, re-exported from the crate root:
//!
//! ```text
//! #[derive(Clone, Debug, PartialEq)]
//! pub struct ScriptCommand { pub name: String, pub args: [f32; 3] }
//! impl ScriptRuntime {
//!     /// Takes the commands queued with `emit_command` since the last call (oldest first).
//!     pub fn take_commands(&mut self, id: InstanceId) -> Vec<ScriptCommand>;
//!     /// Sets many named inputs at once (same as `set_input` for each pair). False for an unknown
//!     /// instance (nothing is set).
//!     pub fn set_inputs<'a>(&mut self, id: InstanceId, values: impl IntoIterator<Item = (&'a str, f32)>) -> bool;
//! }
//! ```
//!
//! Host function (module `"env"`), permission `api.events` (commands are outgoing messages like events):
//!   * `emit_command(name_ptr: i32, name_len: i32, a: f32, b: f32, c: f32)`
//!
//! Same rules as `emit_event`: a name out of the memory bounds, longer than 64 bytes or not valid UTF-8
//! is a harmless no-op; at most 256 queued commands per instance between two `take_commands` calls
//! (more are dropped); never a trap. Commands and events are separate queues.
//!
//! Where it goes: `HostData` (host.rs) gets the queue next to `events` (same limit constant style), both
//! backends get `host_emit_command` and the import next to `emit_event` in `build_linker`; the
//! `BackendInstance` trait gets `take_commands`.

#![allow(clippy::unwrap_used)]

use redox_script::{
    ApiPermissions, BackendKind, ScriptCommand, ScriptConfig, ScriptError, ScriptRuntime,
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

/// Emits `spawn(1, 2, 3)`, an event `done`, then `move(4, 5, 6)`.
const TWO_COMMANDS: &str = r#"
(module
  (import "env" "emit_command" (func $cmd (param i32 i32 f32 f32 f32)))
  (import "env" "emit_event" (func $evt (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "spawnmovedone")
  (func (export "on_update") (param f32)
    (call $cmd (i32.const 0) (i32.const 5) (f32.const 1) (f32.const 2) (f32.const 3))
    (call $evt (i32.const 9) (i32.const 4) (f32.const 1))
    (call $cmd (i32.const 5) (i32.const 4) (f32.const 4) (f32.const 5) (f32.const 6))))
"#;

#[test]
fn commands_arrive_in_order_with_their_arguments_and_stay_apart_from_events() {
    for kind in backends() {
        let mut rt = runtime(kind);
        let id = rt.instantiate(&wasm(TWO_COMMANDS)).unwrap();
        rt.update(id, 0.016).unwrap();
        let commands = rt.take_commands(id);
        assert_eq!(
            commands,
            vec![
                ScriptCommand {
                    name: "spawn".into(),
                    args: [1.0, 2.0, 3.0]
                },
                ScriptCommand {
                    name: "move".into(),
                    args: [4.0, 5.0, 6.0]
                },
            ],
            "{kind:?}"
        );
        assert!(rt.take_commands(id).is_empty(), "taken once");
        let events = rt.take_events(id);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "done");
    }
}

#[test]
fn unknown_instances_have_no_commands() {
    let mut rt = runtime(BackendKind::Wasmi);
    assert!(rt.take_commands(redox_script::InstanceId(9)).is_empty());
}

#[test]
fn the_events_permission_gates_commands() {
    for kind in backends() {
        let config = ScriptConfig {
            api: ApiPermissions {
                events: false,
                ..ApiPermissions::all()
            },
            ..ScriptConfig::default()
        };
        let mut rt = ScriptRuntime::with_backend(config, kind).unwrap();
        assert!(matches!(
            rt.instantiate(&wasm(TWO_COMMANDS)),
            Err(ScriptError::Link(_))
        ));
    }
}

/// 300 commands in one call, then a command with a bad name (out of bounds) and one with an over-long name.
const FLOOD: &str = r#"
(module
  (import "env" "emit_command" (func $cmd (param i32 i32 f32 f32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "tick")
  (func (export "on_update") (param f32)
    (local $i i32)
    (loop $again
      (call $cmd (i32.const 0) (i32.const 4) (f32.convert_i32_u (local.get $i)) (f32.const 0) (f32.const 0))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $again (i32.lt_u (local.get $i) (i32.const 300))))
    (call $cmd (i32.const 65530) (i32.const 10) (f32.const 0) (f32.const 0) (f32.const 0))
    (call $cmd (i32.const 0) (i32.const 65) (f32.const 0) (f32.const 0) (f32.const 0))
    (call $cmd (i32.const -1) (i32.const 4) (f32.const 0) (f32.const 0) (f32.const 0))))
"#;

#[test]
fn only_256_commands_are_kept_and_bad_names_are_ignored() {
    for kind in backends() {
        let mut rt = runtime(kind);
        let id = rt.instantiate(&wasm(FLOOD)).unwrap();
        rt.update(id, 0.016).unwrap();
        assert!(!rt.is_disabled(id), "{kind:?}");
        let commands = rt.take_commands(id);
        assert_eq!(commands.len(), 256);
        assert_eq!(commands[0].args[0], 0.0);
        assert_eq!(commands[255].args[0], 255.0, "the oldest are kept");
        assert!(commands.iter().all(|c| c.name == "tick"));
        rt.update(id, 0.016).unwrap();
        assert_eq!(
            rt.take_commands(id).len(),
            256,
            "the queue was emptied by the take"
        );
    }
}

/// Stores the sum of the inputs `a`, `b` and `c` as the output `sum`.
const SUM_INPUTS: &str = r#"
(module
  (import "env" "get_f32" (func $get (param i32 i32) (result f32)))
  (import "env" "set_f32" (func $set (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "abcsum")
  (func (export "on_update") (param f32)
    (call $set (i32.const 3) (i32.const 3)
      (f32.add
        (f32.add (call $get (i32.const 0) (i32.const 1)) (call $get (i32.const 1) (i32.const 1)))
        (call $get (i32.const 2) (i32.const 1))))))
"#;

#[test]
fn set_inputs_sets_many_values_and_rejects_unknown_instances() {
    for kind in backends() {
        let mut rt = runtime(kind);
        let id = rt.instantiate(&wasm(SUM_INPUTS)).unwrap();
        assert!(rt.set_inputs(id, [("a", 1.0), ("b", 2.0), ("c", 4.0)]));
        rt.update(id, 0.016).unwrap();
        assert_eq!(rt.read_output(id, "sum"), Some(7.0), "{kind:?}");
        assert!(
            rt.set_inputs(id, [("b", 10.0)]),
            "partial updates keep the rest"
        );
        rt.update(id, 0.016).unwrap();
        assert_eq!(rt.read_output(id, "sum"), Some(15.0));
        assert!(!rt.set_inputs(redox_script::InstanceId(5), [("a", 1.0)]));
    }
}
