# ADR 0006: WASM script runtime (prototype B6)

Status: accepted for the prototype (2026-10-17). Follows ADR 0005 (WASM as the scripting layer).

## Decision

`crates/redox_script` hosts guest modules behind a private trait `ScriptBackend`. Default backend:
**`wasmi` 2.0** (pure Rust interpreter). Optional backend: **`wasmtime` 49** (Cranelift JIT) behind the
cargo feature `wasmtime`, selected with `ScriptRuntime::with_backend(config, BackendKind::Wasmtime)`.
It exists to compare numbers and as an upgrade path, not to ship by default.

Safety model (identical for both backends, covered by `tests/runtime.rs` and `tests/wasmtime_backend.rs`):

* **Fuel** per call (`fuel_per_call`, default 1,000,000): an endless loop ends with `OutOfFuel`.
* **Memory cap** (`memory_limit_bytes`, default 16 MiB): `memory.grow` past the cap returns -1; a module
  that starts above the cap is refused.
* **Permissions**: host imports (`env.log`, `env.time_seconds`) are linked only when permitted; otherwise
  `instantiate` fails with `Link`.
* **Isolation**: a trap or fuel exhaustion disables only that instance (`Disabled` afterwards).

## Measurements

Release build, Windows, one thread, `cargo run --release -p redox_script [--features wasmtime] --example measure`.
Single run on the developer machine; treat as order of magnitude.

| Case | wasmi | wasmtime |
|---|---|---|
| empty `on_update` | 52 ns | 14 ns |
| `on_update` with 1 host call | 77-80 ns | 15 ns |
| `on_update` with 100 host calls | 2.8 us (27 ns per host call) | 0.33 us (3 ns per host call) |
| 1M-iteration integer loop | 9.3-10.2 ms (about 10 ns/iteration) | 0.98 ms (about 1 ns/iteration) |
| instantiate small module | 6-7 us | 441 us |

## Consequences

* Per-call overhead is tiny for both: 1,000 scripts calling `on_update` once per frame cost about 50 us
  with wasmi. Gameplay glue (state machines, triggers, UI logic) fits comfortably.
* wasmi is about 10x slower on compute-heavy loops. A script doing per-frame heavy math belongs in native
  Rust, not WASM. The budget is the fuel limit, not the interpreter speed.
* wasmtime wins on speed but instantiating is about 70x slower and the dependency adds roughly two
  minutes to a clean release build. Reconsider only if a real script workload exceeds the wasmi budget.
* wasmi dispatches with LLVM tail calls by default; without guaranteed tail-call optimisation (dev builds)
  a long guest loop can overflow the host stack, so the crate enables `portable-dispatch` by default
  (slower, always safe). Measured numbers above use it.

## Not done (scope of the prototype)

* Host API beyond `log` and `time_seconds` (entity/ECS access) is plan item B7.
* No module hot-reload, no precompiled-module cache, no per-instance epoch interruption.
* Numbers come from one machine and one run.
