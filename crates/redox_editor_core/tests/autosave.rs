//! Headless tests of the autosave service: manual clock, synchronous writes, temporary folders.

use redox_editor_core::EditorLog;
use redox_editor_core::autosave::{
    AutosavePolicy, AutosaveService, AutosaveSource, ManualClock, TickOutcome, WriteMode,
    autosave_dir, list_autosaves,
};
use redox_editor_core::recovery::{
    RecoveryCheck, RecoveryChoice, RecoveryQuery, SessionLock, apply_recovery, check_recovery,
    read_lock, write_lock,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A level whose state the test controls directly.
struct FakeLevel {
    dirty: bool,
    name: String,
    path: Option<PathBuf>,
    bytes: Vec<u8>,
}

impl AutosaveSource for FakeLevel {
    fn is_dirty(&self) -> bool {
        self.dirty
    }

    fn level_name(&self) -> String {
        self.name.clone()
    }

    fn level_path(&self) -> Option<PathBuf> {
        self.path.clone()
    }

    fn snapshot_bytes(&self) -> Result<Vec<u8>, String> {
        Ok(self.bytes.clone())
    }
}

/// A fresh temporary project folder that is unique per test.
fn temp_project(test: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let root = std::env::temp_dir().join(format!(
        "redox_autosave_{test}_{}_{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("temporary project folder is created");
    root
}

/// Writes the level file that autosave must never touch.
fn write_level_file(root: &Path, content: &[u8]) -> PathBuf {
    let dir = root.join("scenes");
    fs::create_dir_all(&dir).expect("scenes folder is created");
    let path = dir.join("main.scene.ron");
    fs::write(&path, content).expect("level file is written");
    path
}

fn level(path: Option<PathBuf>, bytes: &[u8], dirty: bool) -> FakeLevel {
    FakeLevel {
        dirty,
        name: "main".to_string(),
        path,
        bytes: bytes.to_vec(),
    }
}

fn service(root: &Path, clock: &ManualClock, policy: AutosavePolicy) -> AutosaveService {
    AutosaveService::new(
        root,
        policy,
        Box::new(clock.clone()),
        EditorLog::new(),
        WriteMode::Synchronous,
    )
}

fn numbers(root: &Path) -> Vec<usize> {
    list_autosaves(&autosave_dir(root), "main")
        .into_iter()
        .map(|(number, _)| number)
        .collect()
}

#[test]
fn a_clean_level_is_never_autosaved() {
    let root = temp_project("clean");
    let clock = ManualClock::new(0);
    let mut svc = service(&root, &clock, AutosavePolicy::default());
    let source = level(None, b"(a)", false);

    clock.advance(10_000);
    let outcome = svc.tick(&source, 1_000);

    assert_eq!(outcome, TickOutcome::Idle);
    assert!(numbers(&root).is_empty());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_dirty_level_is_autosaved_once_the_interval_has_passed() {
    let root = temp_project("interval");
    let clock = ManualClock::new(0);
    let mut svc = service(&root, &clock, AutosavePolicy::default());
    let source = level(None, b"(state one)", true);

    assert_eq!(svc.tick(&source, 0), TickOutcome::Idle);
    clock.advance(120);
    let outcome = svc.tick(&source, 0);

    assert!(matches!(outcome, TickOutcome::Queued(_)));
    assert_eq!(numbers(&root), vec![1]);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn enough_transactions_trigger_an_autosave_before_the_interval() {
    let root = temp_project("transactions");
    let clock = ManualClock::new(0);
    let mut svc = service(&root, &clock, AutosavePolicy::default());
    let source = level(None, b"(x)", true);

    let outcome = svc.tick(&source, 25);

    assert!(matches!(outcome, TickOutcome::Queued(_)));
    assert_eq!(numbers(&root), vec![1]);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_disabled_policy_never_autosaves() {
    let root = temp_project("disabled");
    let clock = ManualClock::new(0);
    let policy = AutosavePolicy {
        enabled: false,
        ..AutosavePolicy::default()
    };
    let mut svc = service(&root, &clock, policy);
    let source = level(None, b"(x)", true);

    clock.advance(10_000);

    assert_eq!(svc.tick(&source, 1_000), TickOutcome::Idle);
    assert!(numbers(&root).is_empty());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn five_triggers_keep_the_last_three_snapshots_with_growing_numbers() {
    let root = temp_project("ring");
    let clock = ManualClock::new(0);
    let mut svc = service(&root, &clock, AutosavePolicy::default());

    for step in 0..5u32 {
        clock.advance(120);
        let source = level(None, format!("(step {step})").as_bytes(), true);
        assert!(matches!(svc.tick(&source, 0), TickOutcome::Queued(_)));
    }

    assert_eq!(numbers(&root), vec![3, 4, 5]);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_snapshot_file_holds_exactly_the_bytes_of_the_snapshot() {
    let root = temp_project("content");
    let clock = ManualClock::new(0);
    let mut svc = service(&root, &clock, AutosavePolicy::default());
    let content = b"(entities: [(guid: \"00000000000000000000000000000001\")])";
    let source = level(None, content, true);

    clock.advance(120);
    let TickOutcome::Queued(path) = svc.tick(&source, 0) else {
        panic!("an autosave was expected");
    };

    let written = fs::read(&path).expect("autosave file is readable");
    assert_eq!(written, content);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_original_level_file_is_byte_identical_after_autosave() {
    let root = temp_project("original");
    let original = b"(format_version: 1, entities: [])";
    let level_path = write_level_file(&root, original);
    let clock = ManualClock::new(0);
    let mut svc = service(&root, &clock, AutosavePolicy::default());
    let source = level(Some(level_path.clone()), b"(changed in memory)", true);

    clock.advance(120);
    assert!(matches!(svc.tick(&source, 0), TickOutcome::Queued(_)));

    assert_eq!(fs::read(&level_path).expect("level file is readable"), original);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn background_writes_are_on_disk_after_flush_even_when_the_session_closes() {
    let root = temp_project("background");
    let clock = ManualClock::new(0);
    let mut svc = AutosaveService::new(
        &root,
        AutosavePolicy::default(),
        Box::new(clock.clone()),
        EditorLog::new(),
        WriteMode::Background,
    );
    let source = level(None, b"(background)", true);

    clock.advance(120);
    let TickOutcome::Queued(path) = svc.tick(&source, 0) else {
        panic!("an autosave was expected");
    };
    assert!(svc.flush(), "the queue drains within the timeout");

    assert_eq!(fs::read(&path).expect("autosave file is readable"), b"(background)");
    drop(svc);
    let _ = fs::remove_dir_all(&root);
}

// ----- session lock and recovery (EDT-A4 step 2) -----

/// A lock whose heartbeat was last moved at `heartbeat`.
fn lock_with_heartbeat(heartbeat: u64) -> SessionLock {
    SessionLock {
        pid: 4242,
        started: heartbeat,
        project: "project".to_string(),
        level: "main".to_string(),
        heartbeat,
    }
}

/// Recovery query for level `main` at time `now` with the default interval (120 s).
fn query<'a>(root: &'a Path, level_path: Option<&'a Path>, now: u64) -> RecoveryQuery<'a> {
    RecoveryQuery {
        project_root: root,
        level: "main",
        level_path,
        now,
        interval_secs: 120,
    }
}

/// Validator that accepts anything that starts like a RON tuple.
fn looks_like_ron(bytes: &[u8]) -> bool {
    bytes.starts_with(b"(")
}

/// Lets the file system clock move on so that a later write has a later modification time.
fn pause_for_mtime() {
    std::thread::sleep(Duration::from_millis(30));
}

/// Writes a crashed session: a lock from `heartbeat` and an autosave written after the level.
fn crashed_session(root: &Path, heartbeat: u64, autosave: &[u8]) -> PathBuf {
    let level_path = write_level_file(root, b"(level on disk)");
    pause_for_mtime();
    write_lock(root, &lock_with_heartbeat(heartbeat)).expect("lock is written");
    let clock = ManualClock::new(heartbeat);
    let mut svc = service(root, &clock, AutosavePolicy::default());
    let source = level(Some(level_path.clone()), autosave, true);
    clock.advance(120);
    assert!(matches!(svc.tick(&source, 0), TickOutcome::Queued(_)));
    level_path
}

#[test]
fn a_stale_lock_with_a_newer_autosave_offers_recovery() {
    let root = temp_project("offer");
    let level_path = crashed_session(&root, 1_000, b"(recovered state)");
    let log = EditorLog::new();

    let check = check_recovery(&query(&root, Some(&level_path), 1_300), &looks_like_ron, &log);

    let RecoveryCheck::Offer(offer) = check else {
        panic!("a recovery offer was expected");
    };
    assert_eq!(offer.level, "main");
    assert_eq!(offer.level_path.as_deref(), Some(level_path.as_path()));
    assert_eq!(numbers(&root), vec![1]);
    assert_eq!(
        fs::read(&offer.autosave_path).expect("autosave is readable"),
        b"(recovered state)"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn accepting_recovery_gives_the_autosave_bytes_and_keeps_the_level_path() {
    let root = temp_project("accept");
    let level_path = crashed_session(&root, 1_000, b"(recovered state)");
    let log = EditorLog::new();
    let RecoveryCheck::Offer(offer) =
        check_recovery(&query(&root, Some(&level_path), 1_300), &looks_like_ron, &log)
    else {
        panic!("a recovery offer was expected");
    };

    let bytes = apply_recovery(&root, &offer, RecoveryChoice::Recover, 1_300)
        .expect("recovery is applied");

    assert_eq!(bytes.as_deref(), Some(b"(recovered state)".as_slice()));
    assert_eq!(offer.level_path.as_deref(), Some(level_path.as_path()));
    assert!(read_lock(&root).expect("lock is readable").is_none());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn discarding_recovery_moves_the_autosaves_and_removes_the_lock() {
    let root = temp_project("discard");
    let level_path = crashed_session(&root, 1_000, b"(unwanted)");
    let log = EditorLog::new();
    let RecoveryCheck::Offer(offer) =
        check_recovery(&query(&root, Some(&level_path), 1_300), &looks_like_ron, &log)
    else {
        panic!("a recovery offer was expected");
    };

    let bytes = apply_recovery(&root, &offer, RecoveryChoice::Discard, 1_300)
        .expect("discard is applied");

    assert!(bytes.is_none());
    assert!(numbers(&root).is_empty());
    let moved = autosave_dir(&root).join("discarded").join("1300").join("main.1.scene.ron");
    assert_eq!(fs::read(&moved).expect("moved autosave is readable"), b"(unwanted)");
    assert!(read_lock(&root).expect("lock is readable").is_none());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_fresh_lock_reports_another_session_and_offers_nothing() {
    let root = temp_project("fresh");
    let level_path = crashed_session(&root, 1_000, b"(state)");
    let log = EditorLog::new();

    let check = check_recovery(&query(&root, Some(&level_path), 1_010), &looks_like_ron, &log);

    assert!(matches!(check, RecoveryCheck::LockedByOtherSession(_)));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_damaged_autosave_is_not_offered_and_is_moved_aside() {
    let root = temp_project("damaged");
    let level_path = write_level_file(&root, b"(level on disk)");
    pause_for_mtime();
    write_lock(&root, &lock_with_heartbeat(1_000)).expect("lock is written");
    let dir = autosave_dir(&root);
    fs::create_dir_all(&dir).expect("autosave folder is created");
    fs::write(dir.join("main.1.scene.ron"), b"<<< not a level").expect("damaged file is written");
    let log = EditorLog::new();

    let check = check_recovery(&query(&root, Some(&level_path), 1_300), &looks_like_ron, &log);

    assert_eq!(check, RecoveryCheck::Clean);
    assert!(numbers(&root).is_empty());
    assert!(dir.join("discarded").join("1300").join("main.1.scene.ron").exists());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_stale_lock_without_a_newer_autosave_offers_nothing() {
    let root = temp_project("nothing");
    let level_path = write_level_file(&root, b"(level)");
    write_lock(&root, &lock_with_heartbeat(1_000)).expect("lock is written");
    let log = EditorLog::new();

    let check = check_recovery(&query(&root, Some(&level_path), 1_300), &looks_like_ron, &log);

    assert_eq!(check, RecoveryCheck::Clean);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_clean_close_removes_the_lock_and_the_autosaves() {
    let root = temp_project("clean_close");
    let clock = ManualClock::new(1_000);
    let mut svc = service(&root, &clock, AutosavePolicy::default());
    let source = level(None, b"(session)", true);
    svc.begin_session(&source).expect("session starts");
    clock.advance(120);
    assert!(matches!(svc.tick(&source, 0), TickOutcome::Queued(_)));

    svc.finish_session(false).expect("session ends");

    assert!(read_lock(&root).expect("lock is readable").is_none());
    assert!(numbers(&root).is_empty());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_close_with_unsaved_changes_keeps_the_autosaves_for_recovery() {
    let root = temp_project("dirty_close");
    let clock = ManualClock::new(1_000);
    let mut svc = service(&root, &clock, AutosavePolicy::default());
    let source = level(None, b"(session)", true);
    svc.begin_session(&source).expect("session starts");
    clock.advance(120);
    assert!(matches!(svc.tick(&source, 0), TickOutcome::Queued(_)));

    svc.finish_session(true).expect("session ends");

    assert!(read_lock(&root).expect("lock is readable").is_some());
    assert_eq!(numbers(&root), vec![1]);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_heartbeat_moves_forward_while_the_session_is_active() {
    let root = temp_project("heartbeat");
    let clock = ManualClock::new(1_000);
    let mut svc = service(&root, &clock, AutosavePolicy::default());
    let source = level(None, b"(session)", false);
    svc.begin_session(&source).expect("session starts");

    clock.set(1_006);
    svc.tick(&source, 0);

    let lock = read_lock(&root).expect("lock is readable").expect("lock exists");
    assert_eq!(lock.heartbeat, 1_006);
    assert_eq!(lock.started, 1_000);
    assert_eq!(lock.level, "main");
    let _ = fs::remove_dir_all(&root);
}
