# Examples

Run any example with `cargo run --release --example <name> -p redox_core`.

## For users (start here)
| Example | What it shows | Notes |
|---|---|---|
| `skinned_demo` | clip-driven skinned rigs, two-bone IK, TAA/GTAO/VSM; `--golden` runs the regression frames, `--screenshot` renders headless | also the visual regression test |
| `dino_runner` | 3D endless runner on the full plugin stack | KCC, triggers, spatial SFX |

## Developer / diagnostic
| Example | Purpose |
|---|---|
| `dino_game` | legacy raw-winit loop kept for comparison; prefer `dino_runner` |

All examples are built in CI (`cargo clippy --all-targets`). They are demo code: setup failures
panic (`#![allow(clippy::unwrap_used)]`), and they use winit's closure event loop, which winit 0.30 deprecates.

Older examples (`showcase`, `prism_vault`, `ball_score`, `shadow_lab`) were removed on 2026-10-17; they stay in git history.
