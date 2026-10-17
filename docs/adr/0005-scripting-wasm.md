# ADR 0005: Scripting through WebAssembly

Status: accepted (2026-10-17, owner decision)

## Decision

Game logic that is not written in Rust is written in **WebAssembly** modules, not Lua.
Rust gameplay code stays the primary path (hot reload through a dynamic library remains an option
described in `ENGINE_ASSESSMENT.md`, stage 2); WASM is the embedded, sandboxed layer.

## Why

- WASM is a compilation target for Rust, C, C++, Zig, AssemblyScript and others, so one runtime
  serves several languages instead of one.
- Modules are sandboxed: no file, network or memory access except through the host API. User or
  mod code can be loaded without trusting it.
- Predictable cost and deterministic execution (useful for replays and networking later).
- Modules are plain files: they fit the asset pipeline (`.pak`, GUIDs, hot reload by file change).

## Costs and risks

- Heavier than Lua to embed (a runtime such as `wasmtime` or `wasmi`); crossing the host boundary
  costs more than a Lua call, so the API must be coarse-grained (batched component access).
- No REPL-style authoring; designers need a toolchain (or a thin language on top later).
- Not started: this ADR only fixes the direction. The first step is a prototype that loads a module,
  calls `on_update(dt)` and exposes a small host API (transform read/write, input, events, log).

## Next

Prototype after the game prototype settles the host API shape. Pick the runtime then (`wasmi` for a
small pure-Rust interpreter, `wasmtime` for speed), measure call overhead, and decide the ECS access model.
