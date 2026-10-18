//! Coin pickup written as a WebAssembly gameplay module.
//!
//! The module text is generated for a given number of coins and run by `redox_script`; `game.rs` uses
//! [`ScriptedCoins`] instead of its own distance loop, falling back to that loop if the module fails.
//!
//! Contract of the generated module (host API of `redox_script`, `env` imports `get_f32`, `emit_event`):
//! - Inputs set by the host before every update: `px`, `pz` (player position on the ground plane),
//!   `radius` (pickup radius), and for every coin `i < count` the inputs `cNNx`, `cNNz` where `NN` is
//!   `i` as two decimal digits (`c00x`, `c07z`, `c31x`). Names are 4 bytes each.
//! - `on_update` checks every coin that has not been taken yet: when the distance from the player is
//!   strictly smaller than `radius` the coin is taken and the event `coin` with the value `i` is emitted
//!   (once per coin; a taken coin never fires again). Several coins can fire in one update, in index
//!   order. The distance is `sqrt(dx*dx + dz*dz)`.
//! - At most [`MAX_COINS`] coins. More is an error.
//! - The module keeps its taken flags in an `i32` bit mask global (32 coins fit one mask).

use redox_script::{InstanceId, ScriptConfig, ScriptRuntime};

/// Most coins one module can track (one bit each in a 32 bit mask).
pub const MAX_COINS: usize = 32;

/// Fixed-length step of the game loop; the module's clock only matters to the host, not to pickups.
const UPDATE_DT: f32 = 1.0 / 60.0;

/// Byte offsets of the names in the module's data segment.
const EVENT_NAME_AT: u32 = 0;
const PLAYER_X_AT: u32 = 8;
const PLAYER_Z_AT: u32 = 16;
const RADIUS_AT: u32 = 24;
/// Coin names start here; coin `i` owns 8 bytes: `cNNx` then `cNNz`.
const COIN_NAMES_AT: u32 = 64;

/// The text (WAT) of the pickup module for `count` coins. Returns an error text for `count > MAX_COINS`.
pub fn coin_module_wat(count: usize) -> Result<String, String> {
    if count > MAX_COINS {
        return Err(format!("{count} coins, at most {MAX_COINS} are supported"));
    }
    let names: String = (0..count).map(|i| format!("c{i:02}xc{i:02}z")).collect();
    let checks: String = (0..count).map(coin_check).collect();
    Ok(format!(
        r#"(module
  (import "env" "get_f32" (func $get (param i32 i32) (result f32)))
  (import "env" "emit_event" (func $emit (param i32 i32 f32)))
  (memory (export "memory") 1)
  (data (i32.const {EVENT_NAME_AT}) "coin")
  (data (i32.const {PLAYER_X_AT}) "px")
  (data (i32.const {PLAYER_Z_AT}) "pz")
  (data (i32.const {RADIUS_AT}) "radius")
  (data (i32.const {COIN_NAMES_AT}) "{names}")
  (global $mask (mut i32) (i32.const 0))
  (func (export "on_update") (param f32)
    (local $px f32) (local $pz f32) (local $radius f32) (local $dx f32) (local $dz f32)
    (local.set $px (call $get (i32.const {PLAYER_X_AT}) (i32.const 2)))
    (local.set $pz (call $get (i32.const {PLAYER_Z_AT}) (i32.const 2)))
    (local.set $radius (call $get (i32.const {RADIUS_AT}) (i32.const 6)))
{checks}  ))
"#
    ))
}

/// The WAT that tests coin `i` once: a taken coin is skipped, a coin inside the radius is taken and
/// its event emitted. The distance is compared as `f32.lt` against `radius`.
fn coin_check(i: usize) -> String {
    let bit = 1u32 << i;
    let x_at = COIN_NAMES_AT + 8 * i as u32;
    let z_at = x_at + 4;
    format!(
        r#"    (if (i32.eqz (i32.and (global.get $mask) (i32.const {bit}))) (then
      (local.set $dx (f32.sub (local.get $px) (call $get (i32.const {x_at}) (i32.const 4))))
      (local.set $dz (f32.sub (local.get $pz) (call $get (i32.const {z_at}) (i32.const 4))))
      (if (f32.lt
            (f32.sqrt (f32.add (f32.mul (local.get $dx) (local.get $dx))
                               (f32.mul (local.get $dz) (local.get $dz))))
            (local.get $radius))
        (then
          (global.set $mask (i32.or (global.get $mask) (i32.const {bit})))
          (call $emit (i32.const {EVENT_NAME_AT}) (i32.const 4) (f32.const {i}))))))
"#
    )
}

/// A running pickup module for one level.
pub struct ScriptedCoins {
    runtime: ScriptRuntime,
    id: InstanceId,
    positions: Vec<(f32, f32)>,
    radius: f32,
    taken: usize,
}

impl ScriptedCoins {
    /// Compiles and starts the module for `coins` (x, z positions) with the given pickup `radius`.
    /// Errors (as text) when there are too many coins or the module fails to start.
    pub fn new(coins: &[(f32, f32)], radius: f32) -> Result<Self, String> {
        let text = coin_module_wat(coins.len())?;
        let bytes = wat::parse_str(&text).map_err(|e| e.to_string())?;
        let mut runtime = ScriptRuntime::new(ScriptConfig::default());
        let id = runtime.instantiate(&bytes).map_err(|e| e.to_string())?;
        Ok(Self {
            runtime,
            id,
            positions: coins.to_vec(),
            radius,
            taken: 0,
        })
    }

    /// Moves the player to (`x`, `z`), runs the module once and returns the indices of the coins picked
    /// up by this call (ascending). A failing or disabled module returns an empty list.
    pub fn step(&mut self, x: f32, z: f32) -> Vec<usize> {
        self.runtime.set_input(self.id, "px", x);
        self.runtime.set_input(self.id, "pz", z);
        self.runtime.set_input(self.id, "radius", self.radius);
        for (i, (cx, cz)) in self.positions.iter().enumerate() {
            self.runtime.set_input(self.id, &format!("c{i:02}x"), *cx);
            self.runtime.set_input(self.id, &format!("c{i:02}z"), *cz);
        }
        if self.runtime.update(self.id, UPDATE_DT).is_err() {
            return Vec::new();
        }
        let picked: Vec<usize> = self
            .runtime
            .take_events(self.id)
            .into_iter()
            .filter(|e| e.name == "coin")
            .map(|e| e.value as usize)
            .collect();
        self.taken += picked.len();
        picked
    }

    /// How many coins the module has handed out so far.
    pub fn taken(&self) -> usize {
        self.taken
    }

    /// False once the module trapped or ran out of fuel; it then never picks up coins again.
    pub fn is_live(&self) -> bool {
        !self.runtime.is_disabled(self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RADIUS: f32 = 0.7;

    fn native(coins: &[(f32, f32)], taken: &mut [bool], x: f32, z: f32) -> Vec<usize> {
        let mut picked = Vec::new();
        for (i, (cx, cz)) in coins.iter().enumerate() {
            if !taken[i] && (x - cx).hypot(z - cz) < RADIUS {
                taken[i] = true;
                picked.push(i);
            }
        }
        picked
    }

    #[test]
    fn the_module_text_is_valid_for_every_supported_count() {
        for count in [0, 1, 2, 9, 10, 31, MAX_COINS] {
            let text = coin_module_wat(count).unwrap();
            assert!(wat::parse_str(&text).is_ok(), "count {count}");
        }
        assert!(coin_module_wat(MAX_COINS + 1).is_err());
        assert!(ScriptedCoins::new(&vec![(0.0, 0.0); MAX_COINS + 1], RADIUS).is_err());
    }

    #[test]
    fn coins_are_picked_up_once_in_the_order_of_the_walk() {
        let coins = [(0.0, 3.0), (5.0, 5.0)];
        let mut module = ScriptedCoins::new(&coins, RADIUS).unwrap();
        assert!(module.step(0.0, 0.0).is_empty());
        assert_eq!(module.step(0.0, 3.0), vec![0]);
        assert!(module.step(0.0, 3.0).is_empty(), "a coin counts once");
        assert_eq!(module.taken(), 1);
        assert_eq!(module.step(5.0, 5.2), vec![1]);
        assert_eq!(module.taken(), 2);
    }

    #[test]
    fn two_coins_in_reach_come_out_in_index_order() {
        let coins = [(1.0, 0.0), (1.2, 0.0), (9.0, 9.0)];
        let mut module = ScriptedCoins::new(&coins, RADIUS).unwrap();
        assert_eq!(module.step(1.1, 0.0), vec![0, 1]);
    }

    #[test]
    fn the_radius_is_strict_and_comes_from_the_host() {
        let coins = [(2.0, 0.0)];
        let mut near = ScriptedCoins::new(&coins, 0.5).unwrap();
        assert!(near.step(2.6, 0.0).is_empty(), "0.6 is outside 0.5");
        assert_eq!(near.step(2.4, 0.0), vec![0]);
        let mut wide = ScriptedCoins::new(&coins, 1.0).unwrap();
        assert_eq!(wide.step(2.9, 0.0), vec![0]);
    }

    #[test]
    fn no_coins_is_a_valid_level_for_the_module() {
        let mut module = ScriptedCoins::new(&[], RADIUS).unwrap();
        assert!(module.step(1.0, 1.0).is_empty());
        assert_eq!(module.taken(), 0);
    }

    #[test]
    fn it_matches_the_native_distance_loop_along_a_long_path() {
        // 24 coins on a grid, a deterministic pseudo random walk of 600 steps.
        let coins: Vec<(f32, f32)> = (0..24)
            .map(|i| ((i % 6) as f32 * 3.1 - 7.0, (i / 6) as f32 * 3.7 - 5.0))
            .collect();
        let mut module = ScriptedCoins::new(&coins, RADIUS).unwrap();
        let mut taken = vec![false; coins.len()];
        let (mut x, mut z) = (-8.0f32, -6.0f32);
        let mut state = 12345u32;
        let mut rand = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        let mut total = 0;
        for step in 0..600 {
            x = (x + rand() * 1.6).clamp(-9.0, 9.0);
            z = (z + rand() * 1.6).clamp(-7.0, 7.0);
            let expected = native(&coins, &mut taken, x, z);
            let got = module.step(x, z);
            assert_eq!(got, expected, "step {step} at ({x}, {z})");
            total += got.len();
        }
        assert_eq!(module.taken(), total);
        assert!(total > 0, "the walk should have collected something");
    }

    #[test]
    fn a_full_module_stays_within_the_fuel_budget() {
        let coins: Vec<(f32, f32)> = (0..MAX_COINS).map(|i| (i as f32 * 2.0, 0.0)).collect();
        let mut module = ScriptedCoins::new(&coins, RADIUS).unwrap();
        for _ in 0..100 {
            module.step(-50.0, 0.0);
        }
        assert_eq!(
            module.step(0.0, 0.0),
            vec![0],
            "still alive after 100 updates"
        );
        let _ = ScriptConfig::default();
    }
}
