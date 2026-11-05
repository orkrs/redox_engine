//! Measurements of the asset service on many files (EDT-A7 step 4).
//!
//! Ignored by default. Numbers are printed; there are no thresholds (TZ §18). Run once by the
//! coordinator: `cargo test -p redox_editor --test bench_asset_scan -- --ignored --nocapture`.

#[path = "common/mod.rs"]
mod common;

use std::time::Instant;

use common::EditorHarness;

const COUNTS: [usize; 2] = [1_000, 10_000];
/// Files changed in one batch when measuring event handling.
const BATCH: usize = 1_000;

fn millis(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1_000.0
}

#[test]
#[ignore = "замер: запускает координатор (см. docs/BENCHMARKS.md)"]
fn asset_service_numbers_for_1000_and_10000_empty_files() {
    println!("EDT-A7 замер AssetService (без порогов)");
    for count in COUNTS {
        let mut h = EditorHarness::new();
        let dir = h
            .new_project(&format!("scan_{count}"), "empty")
            .expect("the project is created");
        let started = Instant::now();
        for index in 0..count {
            h.write_file(&format!("assets/bulk/file_{index:05}.bin"), &[]);
        }
        let create_ms = millis(started);

        let started = Instant::now();
        h.open_project(&dir).expect("the project opens with a first scan");
        let first_open_ms = millis(started);
        let counts = h.asset_counts();

        let started = Instant::now();
        h.open_project(&dir).expect("the project opens a second time");
        let reopen_ms = millis(started);

        for index in 0..BATCH {
            h.write_file(&format!("assets/bulk/new_{index:05}.bin"), &[]);
        }
        let started = Instant::now();
        h.tick();
        let batch_created_ms = millis(started);

        for index in 0..BATCH {
            h.touch_asset(&format!("assets/bulk/file_{index:05}.bin"));
        }
        let started = Instant::now();
        h.tick();
        let batch_modified_ms = millis(started);

        println!(
            "files={count} create_ms={create_ms:.1} first_open_ms={first_open_ms:.1} \
             reopen_ms={reopen_ms:.1} created_sidecars={} batch={BATCH} \
             batch_created_tick_ms={batch_created_ms:.1} batch_modified_tick_ms={batch_modified_ms:.1}",
            counts.created
        );
    }
}
