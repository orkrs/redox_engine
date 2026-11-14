# Headless editor CI path (EDT-C4): validate → import → cook on sample projects.
# Used by `.github/workflows/ci.yml` job `editor-pipeline` once EDT-B8/C1 CLI is on main.
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot\..

$Sample = 'crates/redox_editor/tests/data/sample_project'
$Broken = 'crates/redox_editor/tests/data/sample_project_broken'

cargo run -p redox_cook -- validate $Sample --json "$Sample/validate.json"
cargo run -p redox_cook -- import $Sample
cargo run -p redox_cook -- cook $Sample --profile default --json "$Sample/cook_report.json"

cargo run -p redox_cook -- validate $Broken --json "$Broken/validate.json"
if ($LASTEXITCODE -eq 0) {
    throw 'expected non-zero exit from validate on sample_project_broken'
}