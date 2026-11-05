//! Guards atomic writes of editor state (EDT-A4): the sources of every `redox_editor*` crate must
//! not call `std::fs::write` or `File::create` directly. Editor files go through
//! `redox_scene::ofpa::write_atomic` (temporary file, backup, rename).
//!
//! Exemptions: the code after a `#[cfg(test)]` marker in a file (test modules sit at the end of a
//! file), and the files listed in [`ALLOWED`] with the reason they need a raw write.

use std::fs;
use std::path::{Path, PathBuf};

/// Files whose raw writes are allowed, with the reason.
const ALLOWED: [(&str, &str); 1] = [(
    "packager.rs",
    "build artifacts (bundles, reports) written to the package folder, not editor state",
)];

/// Call patterns that bypass `write_atomic`.
const FORBIDDEN: [&str; 2] = ["fs::write(", "File::create("];

/// `src` folders of every editor crate (`redox_editor` and `redox_editor_*`).
fn editor_source_dirs() -> Vec<PathBuf> {
    let crates_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let entries = fs::read_dir(&crates_dir).expect("the crates folder is readable");
    let mut dirs = Vec::new();
    for entry in entries {
        let path = entry.expect("crates folder entry is readable").path();
        let is_editor = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("redox_editor"));
        let src = path.join("src");
        if is_editor && src.is_dir() {
            dirs.push(src);
        }
    }
    dirs
}

/// Every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).expect("source folder is readable");
    for entry in entries {
        let path = entry.expect("source entry is readable").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Lines of `text` that call a forbidden writer, ignoring comments and everything after the
/// first `#[cfg(test)]` marker.
fn violations(path: &Path, text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("#[cfg(test)]") {
            break;
        }
        if trimmed.starts_with("//") {
            continue;
        }
        for pattern in FORBIDDEN {
            if line.contains(pattern) {
                found.push(format!("{}:{}: {}", path.display(), index + 1, trimmed));
            }
        }
    }
    found
}

#[test]
fn editor_state_is_written_only_through_write_atomic() {
    let mut files = Vec::new();
    for dir in editor_source_dirs() {
        rust_files(&dir, &mut files);
    }
    assert!(!files.is_empty(), "no editor sources were found");

    let mut found = Vec::new();
    for file in files {
        let name = file
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        if ALLOWED.iter().any(|(allowed, _)| *allowed == name) {
            continue;
        }
        let text = fs::read_to_string(&file).expect("source file is readable");
        found.extend(violations(&file, &text));
    }

    assert!(
        found.is_empty(),
        "raw file writes in editor sources, use redox_scene::ofpa::write_atomic:\n{}",
        found.join("\n")
    );
}
