//! Tests for `ScriptWatcher` (SCR-03): the polling watcher reloads a script when its `.wasm` file changes.

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use redox_script::{ScriptRuntime, ScriptWatcher, WatchEvent};

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

/// Same but `out = in + 100`; the extra comment makes the file a different size from `V1`.
const V2: &str = r#"
(module
  ;; second version of the script
  (import "env" "get_f32" (func $g (param i32 i32) (result f32)))
  (import "env" "set_f32" (func $s (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "inout")
  (func (export "on_update") (param f32)
    (call $s (i32.const 2) (i32.const 3)
      (f32.add (call $g (i32.const 0) (i32.const 2)) (f32.const 100)))))
"#;

fn temp_wasm(wat_text: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "redox_watch_{}_{}.wasm",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, wat::parse_str(wat_text).unwrap()).unwrap();
    path
}

fn out_after_update(rt: &mut ScriptRuntime, id: redox_script::InstanceId) -> f32 {
    rt.set_input(id, "in", 5.0);
    rt.update(id, 0.016).unwrap();
    rt.read_output(id, "out").unwrap()
}

fn start() -> (
    PathBuf,
    ScriptRuntime,
    redox_script::InstanceId,
    ScriptWatcher,
) {
    let path = temp_wasm(V1);
    let mut rt = ScriptRuntime::new(Default::default());
    let id = rt.instantiate(&std::fs::read(&path).unwrap()).unwrap();
    let watcher = ScriptWatcher::new(path.clone(), id);
    (path, rt, id, watcher)
}

#[test]
fn the_first_poll_does_not_reload() {
    let (path, mut rt, _id, mut watcher) = start();
    assert_eq!(watcher.poll(&mut rt), WatchEvent::Unchanged);
    std::fs::remove_file(path).ok();
}

#[test]
fn a_changed_file_is_reloaded_and_runs_the_new_code() {
    let (path, mut rt, id, mut watcher) = start();
    assert_eq!(out_after_update(&mut rt, id), 6.0);
    std::fs::write(&path, wat::parse_str(V2).unwrap()).unwrap();
    assert_eq!(watcher.poll(&mut rt), WatchEvent::Reloaded);
    assert_eq!(out_after_update(&mut rt, id), 105.0);
    assert_eq!(watcher.poll(&mut rt), WatchEvent::Unchanged, "once");
    std::fs::remove_file(path).ok();
}

#[test]
fn a_broken_file_fails_once_and_keeps_the_old_module() {
    let (path, mut rt, id, mut watcher) = start();
    std::fs::write(&path, b"this is not wasm at all").unwrap();
    assert!(matches!(watcher.poll(&mut rt), WatchEvent::Failed(_)));
    assert_eq!(
        watcher.poll(&mut rt),
        WatchEvent::Unchanged,
        "reported once"
    );
    assert_eq!(out_after_update(&mut rt, id), 6.0, "old module still runs");
    std::fs::remove_file(path).ok();
}

#[test]
fn a_missing_file_is_unchanged() {
    let (path, mut rt, _id, mut watcher) = start();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(watcher.poll(&mut rt), WatchEvent::Unchanged);
}

#[test]
fn a_file_that_comes_back_after_deletion_is_reloaded() {
    let (path, mut rt, id, mut watcher) = start();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(watcher.poll(&mut rt), WatchEvent::Unchanged);
    std::fs::write(&path, wat::parse_str(V2).unwrap()).unwrap();
    assert_eq!(watcher.poll(&mut rt), WatchEvent::Reloaded);
    assert_eq!(out_after_update(&mut rt, id), 105.0);
    std::fs::remove_file(path).ok();
}
