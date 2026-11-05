//! Stress (EDT-A7 step 4): a thousand random operations, then a full undo must give back the
//! state the test started from. The seed is fixed and printed when the check fails.

use crate::common::EditorHarness;

const SEED: u64 = 0x5EED_A7_0001;
const OPERATIONS: usize = 1_000;
const BASE_ENTITIES: usize = 12;

/// A small xorshift generator. Deterministic for a given seed.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound as u64) as usize
    }
}

#[test]
fn a_thousand_random_operations_undo_back_to_the_start() {
    let mut rng = Rng(SEED);
    let mut h = EditorHarness::new();
    h.new_project("stress", "empty")
        .expect("the project is created");
    h.new_scene("scenes/stress.scene.ron")
        .expect("the scene is created");
    for index in 0..BASE_ENTITIES {
        h.create_entity(&format!("base {index}"))
            .expect("a base entity is created");
    }
    h.clear_history();
    let start = h.snapshot();

    for step in 0..OPERATIONS {
        apply_random_operation(&h, &mut rng, step);
    }

    h.undo_all();
    assert_eq!(
        h.snapshot(),
        start,
        "after a full undo the world equals the start (seed {SEED:#x})"
    );
}

fn apply_random_operation(h: &EditorHarness, rng: &mut Rng, step: usize) {
    let guids = h.all_guids();
    let pick = if guids.is_empty() {
        None
    } else {
        Some(guids[rng.below(guids.len())])
    };
    match (rng.below(7), pick) {
        (0, _) | (_, None) => {
            let _ = h.create_entity(&format!("random {step}"));
        }
        (1, Some(guid)) => {
            let _ = h.duplicate(guid);
        }
        (2, Some(guid)) => {
            let _ = h.delete(guid);
        }
        (3, Some(guid)) => {
            let _ = h.rename(guid, &format!("renamed {step}"));
        }
        (4, Some(guid)) => {
            let other = guids[rng.below(guids.len())];
            if other != guid {
                // A cycle is refused by the editor; the refusal leaves the world unchanged.
                let _ = h.reparent(guid, Some(other));
            }
        }
        (5, Some(_)) => {
            h.undo();
        }
        (_, Some(_)) => {
            h.redo();
        }
    }
}
