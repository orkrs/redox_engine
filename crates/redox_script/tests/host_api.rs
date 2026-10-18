//! Specification (tests first) for the script host API v1a (plan items B7, first part, and the
//! `.pak` loading left over from B6).
//!
//! API to add to `redox_script` (re-exported from the crate root). Existing types keep their
//! current shape: `Permissions { log, time }` must NOT get new fields (the B6 spec builds it with a
//! struct literal), the new switches live in a separate struct on `ScriptConfig`.
//!
//! ```text
//! #[derive(Clone, Copy, Debug, PartialEq, Eq)]
//! pub struct ApiPermissions { pub state: bool, pub events: bool }
//! impl ApiPermissions { pub const fn all() -> Self; pub const fn none() -> Self }
//! // ScriptConfig gets a new field `pub api: ApiPermissions` (Default: all()).
//!
//! #[derive(Clone, Debug, PartialEq)]
//! pub struct ScriptEvent { pub name: String, pub value: f32 }
//!
//! impl ScriptRuntime {
//!     /// Host -> script: sets a named input value. False for an unknown instance.
//!     pub fn set_input(&mut self, id: InstanceId, name: &str, value: f32) -> bool;
//!     /// Script -> host: the latest value the script stored under `name` with `set_f32`.
//!     pub fn read_output(&self, id: InstanceId, name: &str) -> Option<f32>;
//!     /// Script -> host: takes the events queued with `emit_event` since the last call (oldest first).
//!     pub fn take_events(&mut self, id: InstanceId) -> Vec<ScriptEvent>;
//!     /// Reads `path` from an asset source (a folder or a `.pak`, see `redox_asset::AssetSource`)
//!     /// and instantiates it. Read failures are `ScriptError::Asset(String)` (new variant).
//!     pub fn instantiate_from_source(&mut self, source: &dyn redox_asset::AssetSource, path: &str)
//!         -> Result<InstanceId, ScriptError>;
//! }
//! ```
//!
//! Host functions imported from module `"env"`; each name argument is `(ptr: i32, len: i32)` into the
//! module's exported `memory`, UTF-8:
//!   * `get_f32(name_ptr, name_len) -> f32`          permission `api.state`;  0.0 when the input was never set
//!   * `set_f32(name_ptr, name_len, value: f32)`     permission `api.state`
//!   * `emit_event(name_ptr, name_len, value: f32)`  permission `api.events`
//!
//! Robustness: a name that is out of the memory bounds, longer than 64 bytes or not valid UTF-8 makes the
//! call a harmless no-op (`get_f32` returns 0.0): no trap, the instance stays enabled. At most 256
//! distinct outputs and 256 queued events per instance; anything beyond is dropped. Importing a host
//! function that is not permitted is a `Link` error, as for `log` and `time_seconds`.

#![allow(clippy::unwrap_used)]

use redox_asset::{AssetSource, DirSource, PakSource, PakWriter};
use redox_script::{ApiPermissions, ScriptConfig, ScriptError, ScriptEvent, ScriptRuntime};

fn wasm(text: &str) -> Vec<u8> {
    wat::parse_str(text).expect("valid wat")
}

fn runtime() -> ScriptRuntime {
    ScriptRuntime::new(ScriptConfig::default())
}

/// distance = speed * 2
const DOUBLER: &str = r#"
(module
  (import "env" "get_f32" (func $get (param i32 i32) (result f32)))
  (import "env" "set_f32" (func $set (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "speed")
  (data (i32.const 16) "distance")
  (func (export "on_update") (param f32)
    (call $set (i32.const 16) (i32.const 8)
      (f32.mul (call $get (i32.const 0) (i32.const 5)) (f32.const 2)))))
"#;

/// The Courier coin rule as a module: emit `coin_collected` (value = coin x) while the player is
/// closer than 1 m to the coin.
const COIN: &str = r#"
(module
  (import "env" "get_f32" (func $get (param i32 i32) (result f32)))
  (import "env" "emit_event" (func $emit (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "player_x")
  (data (i32.const 16) "coin_x")
  (data (i32.const 32) "coin_collected")
  (func (export "on_update") (param f32)
    (if (f32.lt
          (f32.abs (f32.sub (call $get (i32.const 0) (i32.const 8))
                            (call $get (i32.const 16) (i32.const 6))))
          (f32.const 1.0))
      (then (call $emit (i32.const 32) (i32.const 14) (call $get (i32.const 16) (i32.const 6)))))))
"#;

#[test]
fn inputs_reach_the_script_and_outputs_come_back() {
    let mut rt = runtime();
    let id = rt.instantiate(&wasm(DOUBLER)).unwrap();
    assert!(rt.set_input(id, "speed", 3.0));
    rt.update(id, 0.016).unwrap();
    assert_eq!(rt.read_output(id, "distance"), Some(6.0));
    assert_eq!(rt.read_output(id, "nothing"), None);
    rt.set_input(id, "speed", -1.5);
    rt.update(id, 0.016).unwrap();
    assert_eq!(rt.read_output(id, "distance"), Some(-3.0));
}

#[test]
fn an_input_that_was_never_set_reads_as_zero() {
    let mut rt = runtime();
    let id = rt.instantiate(&wasm(DOUBLER)).unwrap();
    rt.update(id, 0.016).unwrap();
    assert_eq!(rt.read_output(id, "distance"), Some(0.0));
}

#[test]
fn state_is_per_instance() {
    let mut rt = runtime();
    let a = rt.instantiate(&wasm(DOUBLER)).unwrap();
    let b = rt.instantiate(&wasm(DOUBLER)).unwrap();
    rt.set_input(a, "speed", 1.0);
    rt.set_input(b, "speed", 10.0);
    rt.update(a, 0.016).unwrap();
    rt.update(b, 0.016).unwrap();
    assert_eq!(rt.read_output(a, "distance"), Some(2.0));
    assert_eq!(rt.read_output(b, "distance"), Some(20.0));
}

#[test]
fn a_coin_module_emits_an_event_only_when_the_player_is_close() {
    let mut rt = runtime();
    let id = rt.instantiate(&wasm(COIN)).unwrap();
    rt.set_input(id, "coin_x", 10.0);
    rt.set_input(id, "player_x", 0.0);
    rt.update(id, 0.016).unwrap();
    assert!(rt.take_events(id).is_empty(), "far away");

    rt.set_input(id, "player_x", 9.5);
    rt.update(id, 0.016).unwrap();
    assert_eq!(
        rt.take_events(id),
        vec![ScriptEvent {
            name: "coin_collected".into(),
            value: 10.0
        }]
    );
    assert!(rt.take_events(id).is_empty(), "events are taken once");
}

#[test]
fn events_keep_their_order() {
    let two = wasm(
        r#"(module
          (import "env" "emit_event" (func $emit (param i32 i32 f32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "first") (data (i32.const 8) "second")
          (func (export "on_update") (param f32)
            (call $emit (i32.const 0) (i32.const 5) (f32.const 1))
            (call $emit (i32.const 8) (i32.const 6) (f32.const 2))))"#,
    );
    let mut rt = runtime();
    let id = rt.instantiate(&two).unwrap();
    rt.update(id, 0.016).unwrap();
    let names: Vec<_> = rt.take_events(id).into_iter().map(|e| e.name).collect();
    assert_eq!(names, vec!["first", "second"]);
}

#[test]
fn bad_names_are_harmless() {
    // Out-of-bounds pointer, oversized length and invalid UTF-8 (0xFF bytes).
    let module = wasm(
        r#"(module
          (import "env" "get_f32" (func $get (param i32 i32) (result f32)))
          (import "env" "set_f32" (func $set (param i32 i32 f32)))
          (import "env" "emit_event" (func $emit (param i32 i32 f32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "\ff\ff\ff\ff")
          (global $got (export "got") (mut f32) (f32.const -1))
          (func (export "on_update") (param f32)
            (call $set (i32.const 70000) (i32.const 4) (f32.const 1))
            (call $set (i32.const 0) (i32.const 4) (f32.const 1))
            (call $set (i32.const 0) (i32.const 100000) (f32.const 1))
            (call $emit (i32.const 70000) (i32.const 4) (f32.const 1))
            (call $emit (i32.const 0) (i32.const 4) (f32.const 1))
            (global.set $got (call $get (i32.const 70000) (i32.const 4)))))"#,
    );
    let mut rt = runtime();
    let id = rt.instantiate(&module).unwrap();
    rt.update(id, 0.016).unwrap();
    assert!(!rt.is_disabled(id));
    assert!(rt.take_events(id).is_empty());
    assert_eq!(rt.read_global_f32(id, "got"), Some(0.0));
}

#[test]
fn queues_are_bounded() {
    // Emits 1000 events in one update.
    let spam = wasm(
        r#"(module
          (import "env" "emit_event" (func $emit (param i32 i32 f32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "spam")
          (func (export "on_update") (param f32) (local $i i32)
            (loop $again
              (call $emit (i32.const 0) (i32.const 4) (f32.const 1))
              (local.set $i (i32.add (local.get $i) (i32.const 1)))
              (br_if $again (i32.lt_u (local.get $i) (i32.const 1000))))))"#,
    );
    let mut rt = runtime();
    let id = rt.instantiate(&spam).unwrap();
    rt.update(id, 0.016).unwrap();
    assert_eq!(rt.take_events(id).len(), 256);
}

#[test]
fn the_new_imports_need_their_permissions() {
    let mut no_state = ScriptRuntime::new(ScriptConfig {
        api: ApiPermissions {
            state: false,
            events: true,
            ..ApiPermissions::none()
        },
        ..ScriptConfig::default()
    });
    assert!(matches!(
        no_state.instantiate(&wasm(DOUBLER)),
        Err(ScriptError::Link(_))
    ));
    // COIN reads inputs too.
    assert!(matches!(
        no_state.instantiate(&wasm(COIN)),
        Err(ScriptError::Link(_))
    ));

    let mut none = ScriptRuntime::new(ScriptConfig {
        api: ApiPermissions::none(),
        ..ScriptConfig::default()
    });
    assert!(matches!(
        none.instantiate(&wasm(DOUBLER)),
        Err(ScriptError::Link(_))
    ));
    assert!(runtime().instantiate(&wasm(DOUBLER)).is_ok());
    assert!(ApiPermissions::all().state && ApiPermissions::all().events);
    assert!(!ApiPermissions::none().state && !ApiPermissions::none().events);
}

#[test]
fn unknown_instances_do_not_panic() {
    let mut rt = runtime();
    let ghost = redox_script::InstanceId(999);
    assert!(!rt.set_input(ghost, "x", 1.0));
    assert_eq!(rt.read_output(ghost, "x"), None);
    assert!(rt.take_events(ghost).is_empty());
}

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("redox_script_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn modules_load_from_a_folder_source() {
    let dir = scratch_dir("dir");
    std::fs::create_dir_all(dir.join("scripts")).unwrap();
    std::fs::write(dir.join("scripts/doubler.wasm"), wasm(DOUBLER)).unwrap();
    let source = DirSource::new(&dir);
    let mut rt = runtime();
    let id = rt
        .instantiate_from_source(&source, "scripts/doubler.wasm")
        .unwrap();
    rt.set_input(id, "speed", 4.0);
    rt.update(id, 0.016).unwrap();
    assert_eq!(rt.read_output(id, "distance"), Some(8.0));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn modules_load_from_a_pak() {
    let dir = scratch_dir("pak");
    let mut writer = PakWriter::new();
    writer.add("scripts/coin.wasm", wasm(COIN)).unwrap();
    let pak_path = dir.join("game.pak");
    writer.write_to(&pak_path).unwrap();
    let pak = PakSource::open(&pak_path).unwrap();
    let source: &dyn AssetSource = &pak;
    let mut rt = runtime();
    let id = rt
        .instantiate_from_source(source, "scripts/coin.wasm")
        .unwrap();
    rt.set_input(id, "coin_x", 1.0);
    rt.set_input(id, "player_x", 1.2);
    rt.update(id, 0.016).unwrap();
    assert_eq!(rt.take_events(id).len(), 1);
    drop(pak);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_missing_file_is_an_asset_error() {
    let dir = scratch_dir("missing");
    let source = DirSource::new(&dir);
    let mut rt = runtime();
    assert!(matches!(
        rt.instantiate_from_source(&source, "nope.wasm"),
        Err(ScriptError::Asset(_))
    ));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_file_that_is_not_wasm_is_a_compile_error() {
    let dir = scratch_dir("junk");
    std::fs::write(dir.join("junk.wasm"), b"not wasm").unwrap();
    let source = DirSource::new(&dir);
    assert!(matches!(
        runtime().instantiate_from_source(&source, "junk.wasm"),
        Err(ScriptError::Compile(_))
    ));
    let _ = std::fs::remove_dir_all(dir);
}
