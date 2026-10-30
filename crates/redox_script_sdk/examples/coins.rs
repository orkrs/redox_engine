//! Coin pickup written in Rust with `redox_script_sdk`, the same gameplay as the WAT module in
//! `redox_core/examples/courier/scripted_coins.rs`.
//!
//! Build: `cargo build -p redox_script_sdk --example coins --target wasm32-unknown-unknown --release`.
//!
//! Contract: inputs `px`, `pz` (player on the ground plane) and `radius` are set before every
//! update. A coin that is not taken yet and lies strictly inside the radius is taken once: the
//! event `coin` is emitted with the coin index, and the system command `sys.spawn.sparkle` is
//! emitted at the coin position (the host ignores it when the prefab is not registered).
//! The radius test compares squared distances, so it needs no square root (`no_std`).

#![cfg_attr(target_arch = "wasm32", no_std)]

use core::sync::atomic::{AtomicU32, Ordering};

/// Coin positions on the ground plane (x, z).
const COINS: [(f32, f32); 3] = [(2.0, 0.0), (5.0, 1.0), (-3.0, 4.0)];

/// One bit per coin: set when the coin has been taken. Each instance has its own memory, so the
/// state belongs to this script only.
static TAKEN: AtomicU32 = AtomicU32::new(0);

redox_script_sdk::script!(update);

fn update(_dt: f32) {
    let (px, pz) = (redox_script_sdk::input("px"), redox_script_sdk::input("pz"));
    let radius = redox_script_sdk::input("radius");
    if radius <= 0.0 {
        return;
    }
    for (index, &(x, z)) in COINS.iter().enumerate() {
        let bit = 1u32 << index;
        if TAKEN.load(Ordering::Relaxed) & bit != 0 {
            continue;
        }
        let (dx, dz) = (px - x, pz - z);
        if dx * dx + dz * dz < radius * radius {
            TAKEN.fetch_or(bit, Ordering::Relaxed);
            redox_script_sdk::emit("coin", index as f32);
            redox_script_sdk::system_command("spawn.sparkle", x, 0.0, z);
        }
    }
}

#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    core::arch::wasm32::unreachable()
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
