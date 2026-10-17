# Examples

Run any example with `cargo run --release --example <name> -p redox_core`.

## For users (start here)
| Example | What it shows | Notes |
|---|---|---|
| `showcase` | PBR, VSM sun shadows, point-light shadows, SSAO, egui stats, orbiting camera | simplest renderer tour |
| `skinned_demo` | clip-driven skinned rigs, two-bone IK, TAA/GTAO/VSM; `--golden` runs the regression frames, `--screenshot` renders headless | also the visual regression test |
| `ball_score` | small physics game: roll a ball, collect orbs | input + physics + audio |
| `dino_runner` | 3D endless runner on the full plugin stack | KCC, triggers, spatial SFX |
| `prism_vault` | mini-game: crystals, sentinels with StateTree AI, streamed world | `examples/prism_vault/` is split into modules |

## Developer / diagnostic
| Example | Purpose |
|---|---|
| `shadow_lab` | shadow artefact lab (acne, peter-panning, clipmap seams); keys 1/2 toggle modes |
| `dino_game` | legacy raw-winit loop kept for comparison; prefer `dino_runner` |

All examples are built in CI (`cargo clippy --all-targets`). They are demo code: setup failures
panic (`#![allow(clippy::unwrap_used)]`), and they use winit's closure event loop, which winit 0.30 deprecates.
