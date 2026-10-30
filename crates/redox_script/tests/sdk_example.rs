//! Runs the Rust example of `redox_script_sdk` (`examples/coins.rs`) built for `wasm32`.
//!
//! The module is `target/wasm32-unknown-unknown/release/examples/coins.wasm`, written by
//! `cargo build -p redox_script_sdk --example coins --target wasm32-unknown-unknown --release`.
//! Without the file (CI without the wasm target) the test returns early and reports nothing.

#![allow(clippy::unwrap_used)]

use redox_script::{ScriptConfig, ScriptRuntime};

const COINS_WASM: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../target/wasm32-unknown-unknown/release/examples/coins.wasm"
);

#[test]
fn sdk_coins_example_takes_a_coin_once_and_spawns_a_sparkle() {
    let Ok(bytes) = std::fs::read(COINS_WASM) else {
        return;
    };
    let mut runtime = ScriptRuntime::new(ScriptConfig::default());
    let id = runtime.instantiate(&bytes).unwrap();
    // The player stands on coin 0 at (2, 0).
    runtime.set_input(id, "px", 2.0);
    runtime.set_input(id, "pz", 0.0);
    runtime.set_input(id, "radius", 0.5);
    runtime.update(id, 1.0 / 60.0).unwrap();

    let events = runtime.take_events(id);
    assert_eq!(events.len(), 1, "one coin taken: {events:?}");
    assert_eq!(events[0].name, "coin");
    assert_eq!(events[0].value, 0.0);

    let commands = runtime.take_commands(id);
    assert_eq!(commands.len(), 1, "one sparkle spawned: {commands:?}");
    assert_eq!(commands[0].name, "sys.spawn.sparkle");
    assert_eq!(commands[0].args, [2.0, 0.0, 0.0]);

    // The taken coin never fires again.
    runtime.update(id, 1.0 / 60.0).unwrap();
    assert!(runtime.take_events(id).is_empty());
}
