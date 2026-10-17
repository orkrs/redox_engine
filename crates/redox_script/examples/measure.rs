//! Micro benchmark for the script runtime (ADR 0006).
//!
//! Run in release mode: `cargo run --release -p redox_script --example measure`.
//! Reports nanoseconds per call for an empty `on_update`, `on_update` with 1 and 100 host calls,
//! a 1,000,000 iteration integer loop, and the time to instantiate a small module.

use std::fmt::Write as _;
use std::hint::black_box;
use std::time::Instant;

use redox_script::{BackendKind, InstanceId, ScriptConfig, ScriptRuntime};

const DT: f32 = 0.016;

fn module_with_body(body: &str) -> Vec<u8> {
    let text = format!(
        r#"(module
  (import "env" "time_seconds" (func $time (result f32)))
  (func (export "on_update") (param f32) (local $i i32) (local $acc i32)
    {body}))"#
    );
    wat::parse_str(text).expect("valid wat")
}

fn calls_module(count: usize) -> Vec<u8> {
    let mut body = String::new();
    for _ in 0..count {
        let _ = writeln!(body, "(drop (call $time))");
    }
    module_with_body(&body)
}

fn compute_module(iterations: u32) -> Vec<u8> {
    module_with_body(&format!(
        r#"(loop $again
      (local.set $acc (i32.xor (i32.mul (local.get $acc) (i32.const 31)) (local.get $i)))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $again (i32.lt_u (local.get $i) (i32.const {iterations}))))"#
    ))
}

fn runtime(kind: BackendKind) -> ScriptRuntime {
    let config = ScriptConfig {
        fuel_per_call: u64::MAX / 2,
        ..ScriptConfig::default()
    };
    ScriptRuntime::with_backend(config, kind).expect("create runtime")
}

/// Average nanoseconds per `update` over `iterations` calls (after a short warm-up).
fn per_update(kind: BackendKind, wasm: &[u8], iterations: u32) -> f64 {
    let mut rt = runtime(kind);
    let id: InstanceId = rt.instantiate(wasm).expect("instantiate");
    for _ in 0..(iterations / 10).max(1) {
        rt.update(id, DT).expect("warm-up update");
    }
    let start = Instant::now();
    for _ in 0..iterations {
        rt.update(black_box(id), black_box(DT)).expect("update");
    }
    start.elapsed().as_nanos() as f64 / f64::from(iterations)
}

/// Average nanoseconds to compile and instantiate `wasm`.
fn per_instantiate(kind: BackendKind, wasm: &[u8], iterations: u32) -> f64 {
    let mut rt = runtime(kind);
    let start = Instant::now();
    for _ in 0..iterations {
        black_box(rt.instantiate(black_box(wasm)).expect("instantiate"));
    }
    start.elapsed().as_nanos() as f64 / f64::from(iterations)
}

fn measure(kind: BackendKind) {
    println!("backend: {kind:?}");
    let empty = module_with_body("");
    let a = per_update(kind, &empty, 200_000);
    let b = per_update(kind, &calls_module(1), 200_000);
    let c = per_update(kind, &calls_module(100), 20_000);
    let d = per_update(kind, &compute_module(1_000_000), 30);
    let e = per_instantiate(kind, &empty, 2_000);
    println!("(a) empty on_update          : {a:>12.1} ns/call");
    println!("(b) on_update, 1 host call   : {b:>12.1} ns/call");
    println!(
        "(c) on_update, 100 host calls: {c:>12.1} ns/call ({:.1} ns/host call)",
        (c - a) / 100.0
    );
    println!(
        "(d) 1M-iteration int loop    : {d:>12.1} ns/call ({:.2} ms, {:.2} ns/iteration)",
        d / 1e6,
        d / 1e6
    );
    println!("(e) instantiate small module : {e:>12.1} ns");
}

fn main() {
    measure(BackendKind::Wasmi);
    #[cfg(feature = "wasmtime")]
    measure(BackendKind::Wasmtime);
}
