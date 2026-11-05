//! Scenario tests of stage A (EDT-A7, acceptance by TZ §23). One test binary, one module per
//! scenario. Every scenario drives the editor only through `common::EditorHarness`.
//!
//! Run by the coordinator: `cargo test -p redox_editor --test scenarios`.

#[path = "../common/mod.rs"]
mod common;

mod s00_harness;
