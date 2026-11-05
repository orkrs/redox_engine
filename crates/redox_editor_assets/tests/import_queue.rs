//! Import queue and import statuses (EDT-A6 step 4). Fixtures are built inside the tests.

use redox_asset::AssetGuid;
use redox_asset::guid::AssetStatus;
use redox_editor_assets::{
    AssetChange, AssetImporter, AssetService, CancelToken, ImportError, ImportEvent, ImportJob,
    ImportOutput, ImportQueue, ImportSettings, ImporterRegistry,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(10);

/// A temporary folder removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "redox_editor_import_{name}_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A PNG file with a valid signature and IHDR chunk (the probe does not check the CRC).
fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend_from_slice(&13u32.to_be_bytes());
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes
}

/// Panics whenever it runs (extension `boom`).
struct PanicImporter;

impl AssetImporter for PanicImporter {
    fn name(&self) -> &str {
        "panic-test"
    }
    fn extensions(&self) -> &[&str] {
        &["boom"]
    }
    fn version(&self) -> u32 {
        1
    }
    fn import(&self, _job: &ImportJob, _cancel: &CancelToken) -> Result<ImportOutput, ImportError> {
        panic!("simulated importer crash");
    }
}

/// Runs until it is cancelled (extension `slow`).
struct SlowImporter;

impl AssetImporter for SlowImporter {
    fn name(&self) -> &str {
        "slow-test"
    }
    fn extensions(&self) -> &[&str] {
        &["slow"]
    }
    fn version(&self) -> u32 {
        1
    }
    fn import(&self, _job: &ImportJob, cancel: &CancelToken) -> Result<ImportOutput, ImportError> {
        while !cancel.is_cancelled() {
            std::thread::sleep(Duration::from_millis(5));
        }
        Err(ImportError::Cancelled)
    }
}

fn test_registry() -> ImporterRegistry {
    let mut registry = ImporterRegistry::with_builtins();
    registry.register(Arc::new(PanicImporter));
    registry.register(Arc::new(SlowImporter));
    registry
}

/// Collects queue events until `done` holds or the wait limit passes.
fn wait_events(
    queue: &ImportQueue,
    events: &mut Vec<ImportEvent>,
    done: impl Fn(&[ImportEvent]) -> bool,
) {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline && !done(events) {
        events.extend(queue.drain_events());
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn outcome_of<'a>(events: &'a [ImportEvent], path: &Path) -> Option<&'a ImportEvent> {
    events.iter().find(|event| match event {
        ImportEvent::Finished { path: p, .. }
        | ImportEvent::Failed { path: p, .. }
        | ImportEvent::Cancelled { path: p, .. } => p.as_path() == path,
        _ => false,
    })
}

fn has_started(events: &[ImportEvent], path: &Path) -> bool {
    events
        .iter()
        .any(|event| matches!(event, ImportEvent::Started { path: p, .. } if p.as_path() == path))
}

fn position_of_start(events: &[ImportEvent], path: &Path) -> Option<usize> {
    events
        .iter()
        .position(|event| matches!(event, ImportEvent::Started { path: p, .. } if p.as_path() == path))
}

#[test]
fn valid_png_is_imported_with_its_importer_version() {
    let dir = TempDir::new("valid");
    let file = dir.path().join("ok.png");
    fs::write(&file, png_bytes(16, 8)).expect("write");
    let queue = ImportQueue::new(ImporterRegistry::with_builtins(), 2);

    assert!(queue.submit(file.clone(), AssetGuid(42)));
    let mut events = Vec::new();
    wait_events(&queue, &mut events, |e| outcome_of(e, &file).is_some());

    match outcome_of(&events, &file) {
        Some(ImportEvent::Finished {
            importer_version,
            report,
            ..
        }) => {
            assert_eq!(*importer_version, 1);
            assert!(report.info.iter().any(|line| line.contains("16x8")));
        }
        other => panic!("expected Finished, got {other:?}"),
    }
}

#[test]
fn broken_file_fails_while_other_files_import_and_the_source_is_untouched() {
    let dir = TempDir::new("broken");
    let broken = dir.path().join("broken.png");
    let ok = dir.path().join("ok.png");
    let garbage = b"definitely not a texture".to_vec();
    fs::write(&broken, &garbage).expect("write broken");
    fs::write(&ok, png_bytes(4, 4)).expect("write ok");
    let queue = ImportQueue::new(ImporterRegistry::with_builtins(), 2);

    assert!(queue.submit(broken.clone(), AssetGuid(1)));
    assert!(queue.submit(ok.clone(), AssetGuid(2)));
    let mut events = Vec::new();
    wait_events(&queue, &mut events, |e| {
        outcome_of(e, &broken).is_some() && outcome_of(e, &ok).is_some()
    });

    match outcome_of(&events, &broken) {
        Some(ImportEvent::Failed { message, .. }) => {
            assert!(message.contains("unknown texture format"), "message: {message}");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert!(matches!(
        outcome_of(&events, &ok),
        Some(ImportEvent::Finished { .. })
    ));
    assert_eq!(fs::read(&broken).expect("read"), garbage, "source must not change");
}

#[test]
fn panicking_importer_does_not_stop_other_jobs() {
    let dir = TempDir::new("panic");
    let boom = dir.path().join("crash.boom");
    let ok = dir.path().join("ok.png");
    fs::write(&boom, b"x").expect("write boom");
    fs::write(&ok, png_bytes(2, 2)).expect("write ok");
    let queue = ImportQueue::new(test_registry(), 1);

    assert!(queue.submit(boom.clone(), AssetGuid(1)));
    assert!(queue.submit(ok.clone(), AssetGuid(2)));
    let mut events = Vec::new();
    wait_events(&queue, &mut events, |e| {
        outcome_of(e, &boom).is_some() && outcome_of(e, &ok).is_some()
    });

    match outcome_of(&events, &boom) {
        Some(ImportEvent::Failed { message, .. }) => {
            assert!(message.contains("panicked"), "message: {message}");
            assert!(message.contains("simulated importer crash"), "message: {message}");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert!(matches!(
        outcome_of(&events, &ok),
        Some(ImportEvent::Finished { .. })
    ));
}

#[test]
fn job_cancelled_before_it_starts_reports_cancelled() {
    let dir = TempDir::new("cancel_queued");
    let blocker = dir.path().join("blocker.slow");
    let waiting = dir.path().join("waiting.slow");
    fs::write(&blocker, b"x").expect("write");
    fs::write(&waiting, b"x").expect("write");
    let queue = ImportQueue::new(test_registry(), 1);

    assert!(queue.submit(blocker.clone(), AssetGuid(1)));
    let mut events = Vec::new();
    wait_events(&queue, &mut events, |e| has_started(e, &blocker));
    assert!(queue.submit(waiting.clone(), AssetGuid(2)));

    assert!(queue.cancel(&waiting));
    assert!(matches!(
        outcome_of(&queue.drain_events(), &waiting),
        Some(ImportEvent::Cancelled { .. })
    ));
    assert!(queue.cancel(&blocker));
    wait_events(&queue, &mut events, |e| outcome_of(e, &blocker).is_some());
    assert!(matches!(
        outcome_of(&events, &blocker),
        Some(ImportEvent::Cancelled { .. })
    ));
}

#[test]
fn job_cancelled_while_running_stops() {
    let dir = TempDir::new("cancel_running");
    let slow = dir.path().join("run.slow");
    fs::write(&slow, b"x").expect("write");
    let queue = ImportQueue::new(test_registry(), 1);

    assert!(queue.submit(slow.clone(), AssetGuid(1)));
    let mut events = Vec::new();
    wait_events(&queue, &mut events, |e| has_started(e, &slow));
    assert!(queue.cancel(&slow));
    wait_events(&queue, &mut events, |e| outcome_of(e, &slow).is_some());

    assert!(matches!(
        outcome_of(&events, &slow),
        Some(ImportEvent::Cancelled { .. })
    ));
}

#[test]
fn retry_after_the_file_is_fixed_imports_it() {
    let dir = TempDir::new("retry");
    let file = dir.path().join("fixme.png");
    fs::write(&file, b"broken").expect("write broken");
    let queue = ImportQueue::new(ImporterRegistry::with_builtins(), 1);

    assert!(queue.submit(file.clone(), AssetGuid(7)));
    let mut events = Vec::new();
    wait_events(&queue, &mut events, |e| outcome_of(e, &file).is_some());
    assert!(matches!(
        outcome_of(&events, &file),
        Some(ImportEvent::Failed { .. })
    ));

    fs::write(&file, png_bytes(8, 8)).expect("fix file");
    assert!(queue.retry(&file));
    let mut again = Vec::new();
    wait_events(&queue, &mut again, |e| outcome_of(e, &file).is_some());
    assert!(matches!(
        outcome_of(&again, &file),
        Some(ImportEvent::Finished { .. })
    ));
}

#[test]
fn open_asset_is_imported_before_other_queued_assets() {
    let dir = TempDir::new("priority");
    let blocker = dir.path().join("blocker.slow");
    let first = dir.path().join("a.png");
    let open = dir.path().join("b.png");
    fs::write(&blocker, b"x").expect("write");
    fs::write(&first, png_bytes(1, 1)).expect("write");
    fs::write(&open, png_bytes(1, 1)).expect("write");
    let queue = ImportQueue::new(test_registry(), 1);

    assert!(queue.submit(blocker.clone(), AssetGuid(1)));
    let mut events = Vec::new();
    wait_events(&queue, &mut events, |e| has_started(e, &blocker));
    assert!(queue.submit(first.clone(), AssetGuid(2)));
    assert!(queue.submit(open.clone(), AssetGuid(3)));
    queue.set_open(Some(open.clone()));
    assert!(queue.cancel(&blocker));
    wait_events(&queue, &mut events, |e| {
        outcome_of(e, &first).is_some() && outcome_of(e, &open).is_some()
    });

    let open_at = position_of_start(&events, &open).expect("open asset started");
    let first_at = position_of_start(&events, &first).expect("other asset started");
    assert!(open_at < first_at, "open asset must run first");
}

/// Polls the service until `done` holds on the collected changes or the wait limit passes.
fn poll_service(
    service: &mut AssetService,
    changes: &mut Vec<AssetChange>,
    done: impl Fn(&[AssetChange]) -> bool,
) {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline && !done(changes) {
        changes.extend(service.tick());
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn imported(changes: &[AssetChange], path: &str) -> bool {
    changes
        .iter()
        .any(|c| matches!(c, AssetChange::Imported { path: p } if p == path))
}

fn settings() -> ImportSettings {
    ImportSettings {
        workers: 2,
        auto_import: true,
    }
}

#[test]
fn outdated_asset_is_imported_when_the_root_is_opened() {
    let dir = TempDir::new("open_import");
    fs::write(dir.path().join("ok.png"), png_bytes(4, 4)).expect("write");
    let mut service = AssetService::open_with(dir.path(), settings()).expect("open");
    let mut changes = Vec::new();

    poll_service(&mut service, &mut changes, |c| imported(c, "ok.png"));

    assert!(imported(&changes, "ok.png"));
    assert_eq!(service.status("ok.png"), Some(&AssetStatus::Imported));
}

#[test]
fn broken_asset_fails_and_other_assets_still_import() {
    let dir = TempDir::new("service_broken");
    fs::write(dir.path().join("broken.png"), b"garbage").expect("write");
    fs::write(dir.path().join("ok.png"), png_bytes(4, 4)).expect("write");
    let mut service = AssetService::open_with(dir.path(), settings()).expect("open");
    let mut changes = Vec::new();

    poll_service(&mut service, &mut changes, |c| {
        imported(c, "ok.png")
            && c.iter()
                .any(|x| matches!(x, AssetChange::ImportFailed { path, .. } if path == "broken.png"))
    });

    assert!(matches!(
        service.status("broken.png"),
        Some(AssetStatus::Failed(_))
    ));
    assert_eq!(service.status("ok.png"), Some(&AssetStatus::Imported));
    assert_eq!(
        fs::read(dir.path().join("broken.png")).expect("read"),
        b"garbage".to_vec()
    );
    let counts = service.import_counts();
    assert_eq!(counts.failed, 1);
}

#[test]
fn changed_file_is_reimported_and_other_files_are_not() {
    let dir = TempDir::new("reimport");
    fs::write(dir.path().join("ok.png"), png_bytes(4, 4)).expect("write");
    fs::write(dir.path().join("other.png"), png_bytes(4, 4)).expect("write");
    let mut service = AssetService::open_with(dir.path(), settings()).expect("open");
    let mut changes = Vec::new();
    poll_service(&mut service, &mut changes, |c| {
        imported(c, "ok.png") && imported(c, "other.png")
    });

    fs::write(dir.path().join("ok.png"), png_bytes(9, 9)).expect("change");
    let mut after = Vec::new();
    poll_service(&mut service, &mut after, |c| imported(c, "ok.png"));

    assert!(imported(&after, "ok.png"), "changed file must be imported again");
    assert!(
        !after
            .iter()
            .any(|c| matches!(c, AssetChange::Imported { path } if path == "other.png")),
        "unchanged file must not be imported again"
    );
}
