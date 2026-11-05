//! Guards the editor crate layout (EDT-A1): which editor crates may depend on egui and on which
//! other editor crates. The manifests are read as plain text; only `[dependencies]` is checked.

use std::fs;
use std::path::PathBuf;

/// Every editor library crate, without the `redox_editor` binary.
const EDITOR_LIBRARIES: [&str; 8] = [
    "redox_editor_core",
    "redox_editor_viewport",
    "redox_editor_debugger",
    "redox_editor_scene",
    "redox_editor_assets",
    "redox_editor_build",
    "redox_editor_plugins",
    "redox_editor_ui",
];

/// Crates that may not depend on the panel crate or on the viewport.
const NO_UI_NO_VIEWPORT: [&str; 5] = [
    "redox_editor_scene",
    "redox_editor_assets",
    "redox_editor_debugger",
    "redox_editor_build",
    "redox_editor_plugins",
];

/// Core must stay free of every other editor crate.
const CORE_FORBIDDEN: [&str; 7] = [
    "redox_editor_ui",
    "redox_editor_scene",
    "redox_editor_assets",
    "redox_editor_viewport",
    "redox_editor_debugger",
    "redox_editor_build",
    "redox_editor_plugins",
];

/// Reads the dependency names listed under `[dependencies]` of `crate_name`.
fn dependencies(crate_name: &str) -> Vec<String> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(crate_name)
        .join("Cargo.toml");
    let text = fs::read_to_string(&manifest)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", manifest.display()));
    let mut names = Vec::new();
    let mut in_dependencies = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            in_dependencies = line == "[dependencies]";
            continue;
        }
        if !in_dependencies || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((name, _)) = line.split_once('=') {
            names.push(name.trim().to_string());
        }
    }
    names
}

/// Dependencies of `crate_name` that are named in `forbidden`.
fn forbidden_dependencies(crate_name: &str, forbidden: &[&str]) -> Vec<String> {
    dependencies(crate_name)
        .into_iter()
        .filter(|name| forbidden.contains(&name.as_str()))
        .collect()
}

/// Whether a dependency name belongs to the egui family (`egui`, `egui-wgpu`, `egui_dock`, ...).
fn is_egui(name: &str) -> bool {
    name == "egui" || name.starts_with("egui-") || name.starts_with("egui_")
}

#[test]
fn egui_is_declared_only_by_the_ui_crate_and_the_binary() {
    for crate_name in EDITOR_LIBRARIES {
        if crate_name == "redox_editor_ui" {
            continue;
        }
        let egui_deps: Vec<String> = dependencies(crate_name)
            .into_iter()
            .filter(|name| is_egui(name))
            .collect();
        assert!(
            egui_deps.is_empty(),
            "{crate_name} must not depend on egui, found {egui_deps:?}"
        );
    }
}

#[test]
fn the_viewport_crate_does_not_depend_on_egui() {
    let egui_deps: Vec<String> = dependencies("redox_editor_viewport")
        .into_iter()
        .filter(|name| is_egui(name))
        .collect();
    assert!(
        egui_deps.is_empty(),
        "viewport depends on egui: {egui_deps:?}"
    );
}

#[test]
fn core_does_not_depend_on_any_other_editor_crate() {
    let found = forbidden_dependencies("redox_editor_core", &CORE_FORBIDDEN);
    assert!(found.is_empty(), "core depends on editor crates: {found:?}");
}

#[test]
fn logic_crates_do_not_depend_on_the_ui_or_the_viewport() {
    for crate_name in NO_UI_NO_VIEWPORT {
        let found =
            forbidden_dependencies(crate_name, &["redox_editor_ui", "redox_editor_viewport"]);
        assert!(
            found.is_empty(),
            "{crate_name} depends on the ui or viewport crate: {found:?}"
        );
    }
}
