//! Guard: every change of the editor world goes through the undo history.
//!
//! The test scans the sources of all `redox_editor*` crates and reports direct calls that create,
//! delete or change components (`.spawn(`, `.despawn(`, `.add_component(`, `.remove_component(`,
//! `insert_component(`) outside the places allowed to make them:
//!
//! - the files in [`ALLOWED_FILES`]: the undo engine itself and the play-in-editor and time-travel
//!   code, which restore world snapshots and keep their own state;
//! - a line that carries one of the markers in [`MARKERS`]: `// undo: tx` for a change that is
//!   recorded in a transaction or as a step, `// undo: init` for the startup scene before any
//!   history exists;
//! - the `#[cfg(test)]` module at the end of a file (test code is not editor code).
//!
//! Comment lines are ignored. Adding a new direct call therefore fails this test until the call
//! is recorded, or until it is marked and the marker is justified in review.

use std::fs;
use std::path::{Path, PathBuf};

/// Calls that change the world without the undo history.
const FORBIDDEN: [&str; 5] = [
    ".spawn(",
    ".despawn(",
    ".add_component(",
    ".remove_component(",
    "insert_component(",
];

/// Markers that allow a line to make a direct change.
const MARKERS: [&str; 2] = ["// undo: tx", "// undo: init"];

/// Files that may change the world directly, as paths relative to the `crates` directory.
const ALLOWED_FILES: [&str; 4] = [
    "redox_editor_core/src/undo.rs",
    "redox_editor_core/src/guid.rs",
    "redox_editor_debugger/src/pie.rs",
    "redox_editor_debugger/src/time_travel.rs",
];

/// Directory that holds all workspace crates (`crates/`).
fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the redox_editor crate lives in crates/")
        .to_path_buf()
}

/// All `.rs` files under the `src` directory of every `redox_editor*` crate.
fn editor_source_files(crates: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = fs::read_dir(crates) else {
        return files;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with("redox_editor") {
            collect_rust_files(&entry.path().join("src"), &mut files);
        }
    }
    files.sort();
    files
}

fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// The path of `file` relative to `crates`, with forward slashes.
fn relative_path(crates: &Path, file: &Path) -> String {
    file.strip_prefix(crates)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
}

/// One-based numbers of the lines in `source` that change the world directly and are not allowed.
fn find_violations(relative: &str, source: &str) -> Vec<usize> {
    if ALLOWED_FILES.iter().any(|allowed| relative.ends_with(allowed)) {
        return Vec::new();
    }
    let mut found = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("#[cfg(test)]") {
            break;
        }
        if trimmed.starts_with("//") {
            continue;
        }
        let direct = FORBIDDEN.iter().any(|pattern| line.contains(pattern));
        let marked = MARKERS.iter().any(|marker| line.contains(marker));
        if direct && !marked {
            found.push(index + 1);
        }
    }
    found
}

#[test]
fn no_direct_world_changes_outside_the_allowed_places() {
    let crates = crates_dir();
    let mut problems = Vec::new();
    for file in editor_source_files(&crates) {
        let Ok(source) = fs::read_to_string(&file) else {
            continue;
        };
        let relative = relative_path(&crates, &file);
        for line in find_violations(&relative, &source) {
            problems.push(format!("{relative}:{line}"));
        }
    }
    assert!(
        problems.is_empty(),
        "direct world changes outside the undo history:\n{}",
        problems.join("\n")
    );
}

#[test]
fn the_guard_flags_a_bare_spawn_and_accepts_marked_and_allowed_ones() {
    let bare = "fn f(w: &mut World) {\n    let e = w.spawn();\n}\n";
    assert_eq!(find_violations("redox_editor/src/app.rs", bare), vec![2]);

    let marked = "fn f(w: &mut World) {\n    let e = w.spawn(); // undo: tx\n}\n";
    assert!(find_violations("redox_editor/src/app.rs", marked).is_empty());

    assert!(find_violations("redox_editor_core/src/undo.rs", bare).is_empty());

    let test_module = "fn f() {}\n#[cfg(test)]\nmod tests {\n    fn g(w: &mut World) { w.spawn(); }\n}\n";
    assert!(find_violations("redox_editor/src/app.rs", test_module).is_empty());

    let comment = "// call w.spawn() later\n";
    assert!(find_violations("redox_editor/src/app.rs", comment).is_empty());
}
