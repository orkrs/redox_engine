# GPU golden frame check for CI (EDT-C4). Requires Vulkan-capable runner.
# Local: see CONTRIBUTING.md for the same command and --update-golden.
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot\..
cargo run --example skinned_demo -p redox_core --release -- --golden crates/redox_core/tests/golden