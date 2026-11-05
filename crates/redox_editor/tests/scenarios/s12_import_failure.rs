//! TZ §23.12: one corrupt file in the import queue fails alone; the others import, the source
//! file is untouched, the editor keeps working, and a retry after the fix succeeds.

use crate::common::{EditorHarness, make_png};

const GOOD_A: &str = "assets/textures/good_a.png";
const GOOD_B: &str = "assets/textures/good_b.png";
const BROKEN: &str = "assets/textures/broken.png";
const BROKEN_BYTES: &[u8] = b"this is not a PNG";

#[test]
fn a_corrupt_import_fails_alone_and_a_retry_after_the_fix_succeeds() {
    let mut h = EditorHarness::new();
    let dir = h.install_sample("sample_project");
    h.open_project(&dir).expect("the sample project opens");

    h.write_file(GOOD_A, &make_png(16, 16));
    h.write_file(GOOD_B, &make_png(32, 8));
    h.write_file(BROKEN, BROKEN_BYTES);
    h.request_import(GOOD_A);
    h.request_import(GOOD_B);
    h.request_import(BROKEN);
    h.wait_imports_idle();

    assert_eq!(h.import_status(GOOD_A).as_deref(), Some("Imported"));
    assert_eq!(h.import_status(GOOD_B).as_deref(), Some("Imported"));
    let broken_status = h.import_status(BROKEN).unwrap_or_default();
    assert!(
        broken_status.starts_with("Failed"),
        "the corrupt file reports Failed: {broken_status}"
    );
    assert_eq!(
        h.read_file(BROKEN),
        BROKEN_BYTES,
        "the source of the failed import is not changed"
    );

    assert!(h.run("create_entity"), "the editor still runs commands");
    assert_eq!(h.entity_count(), 1, "the editor kept working after the failure");

    h.write_file(BROKEN, &make_png(8, 8));
    h.retry_import(BROKEN);
    h.wait_imports_idle();
    assert_eq!(
        h.import_status(BROKEN).as_deref(),
        Some("Imported"),
        "after the fix a retry imports the file"
    );
}
