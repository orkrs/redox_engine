//! Headless tests of the autosave service: manual clock, synchronous writes, temporary folders.

use redox_editor_core::EditorLog;
use redox_editor_core::autosave::{
    AutosavePolicy, AutosaveService, AutosaveSource, ManualClock, TickOutcome, WriteMode,
    autosave_dir, list_autosaves,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

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
