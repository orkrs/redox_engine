//! Measurements of scene save, load, `is_dirty` and `snapshot` (EDT-A7 step 4).
//!
//! Ignored by default. Numbers are printed; there are no thresholds (TZ §18). Run once by the
//! coordinator: `cargo test -p redox_editor --test bench_scene_io -- --ignored --nocapture`.

#[path = "common/mod.rs"]
mod common;

use std::time::Instant;

use common::EditorHarness;

const SIZES: [usize; 3] = [1_000, 10_000, 50_000];
const DIRTY_CALLS: usize = 10_000;
const SCENE: &str = "scenes/bench.scene.ron";

fn millis(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1_000.0
}

#[test]
#[ignore = "замер: запускает координатор (см. docs/BENCHMARKS.md)"]
fn scene_io_numbers_for_1000_10000_and_50000_entities() {
    println!("EDT-A7 замер ввода-вывода сцены (без порогов)");
    for count in SIZES {
        let mut h = EditorHarness::new();
        let dir = h
            .new_project(&format!("bench_{count}"), "empty")
            .expect("the project is created");
        h.new_scene(SCENE).expect("the scene is created");
        h.spawn_bulk(count);

        let started = Instant::now();
        h.save().expect("the scene is saved");
        let save_ms = millis(started);
        let file_bytes = std::fs::metadata(dir.join(SCENE)).map(|m| m.len()).unwrap_or(0);

        let started = Instant::now();
        let snapshot = h.snapshot();
        let snapshot_ms = millis(started);
        let snapshot_entities = snapshot.entities.len();

        let started = Instant::now();
        let mut dirty_calls_true = 0usize;
        for _ in 0..DIRTY_CALLS {
            if h.is_dirty() {
                dirty_calls_true += 1;
            }
        }
        let dirty_ns = started.elapsed().as_nanos() as f64 / DIRTY_CALLS as f64;

        let mut h = h.restart();
        h.open_project(&dir).expect("the project opens again");
        let started = Instant::now();
        h.open_scene(SCENE).expect("the scene opens");
        let load_ms = millis(started);
        let loaded = h.entity_count();

        println!(
            "entities={count} file_bytes={file_bytes} save_ms={save_ms:.1} load_ms={load_ms:.1} \
             snapshot_ms={snapshot_ms:.1} snapshot_entities={snapshot_entities} \
             is_dirty_ns_per_call={dirty_ns:.1} is_dirty_true={dirty_calls_true} loaded={loaded}"
        );
        assert_eq!(loaded, count, "every saved entity is loaded again");
    }
}
