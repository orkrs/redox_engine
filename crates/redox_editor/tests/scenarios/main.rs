//! Scenario tests of stage A (EDT-A7, acceptance by TZ §23). One test binary, one module per
//! scenario. Every scenario drives the editor only through `common::EditorHarness`.
//!
//! Run by the coordinator: `cargo test -p redox_editor --test scenarios`.

#[path = "../common/mod.rs"]
mod common;

mod s00_fixtures;
mod s00_harness;
mod s01_project_roundtrip;
mod s02_scene_roundtrip;
mod s03_edit_undo;
mod s04_undo_stress;
mod s05_asset_rename;
mod s06_references;
mod s08_pie_isolation;
mod s12_import_failure;
mod s13_crash_recovery;
mod s14_broken_project;
mod s16_layout_fallback;
mod s18_editor_only_data;
