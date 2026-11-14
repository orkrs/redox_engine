//! TZ §23.7 — material that fails validation must not be saved (headless).

use redox_editor_assets::{MaterialDocument, MaterialError};
use redox_matgraph::{MaterialGraph, Node, NodeKind, PortRef};
use std::fs;
use std::path::PathBuf;

fn temp() -> PathBuf {
    std::env::temp_dir().join(format!(
        "edt_b5_s07_{}",
        std::process::id()
    ))
}

#[test]
fn s07_invalid_matgraph_is_not_written() {
    let dir = temp();
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("good.matgraph");
    let valid = MaterialGraph {
        version: 1,
        name: "good".into(),
        nodes: vec![
            Node {
                id: 1,
                kind: NodeKind::ConstVec3([0.2, 0.5, 0.9]),
                inputs: vec![],
            },
            Node {
                id: 2,
                kind: NodeKind::SurfaceOutput,
                inputs: vec![Some(PortRef { node: 1, port: 0 }), None, None, None, None, None],
            },
        ],
    };
    let mut doc = MaterialDocument::new("good");
    doc.graph = valid;
    doc.path = Some(path.clone());
    doc.save().expect("seed save");
    let before = fs::read(&path).expect("read");

    let mut open = MaterialDocument::open(&path, None).expect("open");
    open.graph.nodes[1].inputs[0] = None;
    let err = open.save().expect_err("save must fail");
    assert!(matches!(err, MaterialError::SaveBlocked { .. }));
    let after = fs::read(&path).expect("read after");
    assert_eq!(before, after);
}