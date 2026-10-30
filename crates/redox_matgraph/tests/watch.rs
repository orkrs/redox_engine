//! Polling reload of `.matgraph` files (MAT-02 step 6).

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use redox_matgraph::{GraphReload, MatGraphWatcher, MaterialGraph, Node, NodeKind};

/// A valid graph (one constant colour to the output). `name` changes the file size.
fn valid_text(name: &str) -> String {
    MaterialGraph {
        version: 1,
        name: name.into(),
        nodes: vec![
            Node {
                id: 1,
                kind: NodeKind::ConstVec3([1.0, 0.0, 0.0]),
                inputs: vec![],
            },
            Node {
                id: 2,
                kind: NodeKind::SurfaceOutput,
                inputs: vec![
                    Some(redox_matgraph::PortRef { node: 1, port: 0 }),
                    None,
                    None,
                    None,
                    None,
                    None,
                ],
            },
        ],
    }
    .to_ron()
}

/// Fresh file path in the temp directory, unique per test.
fn temp_path(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("matgraph_watch_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{test}.matgraph"))
}

#[test]
fn an_unchanged_file_does_nothing() {
    let path = temp_path("unchanged");
    std::fs::write(&path, valid_text("a")).unwrap();
    let mut watcher = MatGraphWatcher::new(&path);
    assert!(watcher.poll().is_empty());
    assert!(watcher.poll().is_empty());
}

#[test]
fn a_changed_file_triggers_one_reload() {
    let path = temp_path("changed");
    std::fs::write(&path, valid_text("a")).unwrap();
    let mut watcher = MatGraphWatcher::new(&path);
    std::fs::write(&path, valid_text("a much longer graph name")).unwrap();

    let events = watcher.poll();
    assert_eq!(events.len(), 1);
    match &events[0] {
        GraphReload::Ready(graph) => assert_eq!(graph.name, "a much longer graph name"),
        other => panic!("expected a reload, got {other:?}"),
    }
    assert!(watcher.poll().is_empty(), "the change is consumed once");
}

#[test]
fn a_broken_file_reports_once_and_keeps_the_old_variant() {
    let path = temp_path("broken");
    std::fs::write(&path, valid_text("a")).unwrap();
    let mut watcher = MatGraphWatcher::new(&path);
    std::fs::write(&path, "(version: 1, this is not a graph").unwrap();

    let events = watcher.poll();
    assert_eq!(events.len(), 1);
    assert!(matches!(&events[0], GraphReload::Failed(errors) if !errors.is_empty()));
    assert!(
        watcher.poll().is_empty(),
        "the same broken state is not reported again"
    );
}

#[test]
fn a_missing_file_is_silent_until_it_appears() {
    let path = temp_path("appears");
    let _ = std::fs::remove_file(&path);
    let mut watcher = MatGraphWatcher::new(&path);
    assert!(watcher.poll().is_empty());
    std::fs::write(&path, valid_text("late")).unwrap();
    assert!(matches!(watcher.poll().as_slice(), [GraphReload::Ready(_)]));
}
