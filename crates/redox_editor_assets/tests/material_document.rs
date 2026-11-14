//! Headless tests of `MaterialDocument` (EDT-B5 step 1).

use redox_editor_assets::{MaterialDocument, MaterialError, compile_for_save};
use redox_matgraph::{MaterialGraph, Node, NodeKind, PortRef};
use std::fs;
use std::path::PathBuf;

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("edt_b5_matdoc_{name}_{}", std::process::id()))
}

fn link(id: u32) -> Option<PortRef> {
    Some(PortRef { node: id, port: 0 })
}

fn valid_tint_graph() -> MaterialGraph {
    MaterialGraph {
        version: 1,
        name: "tint".into(),
        nodes: vec![
            Node {
                id: 1,
                kind: NodeKind::Param {
                    name: "tint".into(),
                    ty: redox_matgraph::Ty::Vec3,
                    default: [0.8, 0.4, 0.2, 0.0],
                },
                inputs: vec![],
            },
            Node {
                id: 2,
                kind: NodeKind::SurfaceOutput,
                inputs: vec![link(1), None, None, None, None, None],
            },
        ],
    }
}

fn broken_graph() -> MaterialGraph {
    MaterialGraph {
        version: 1,
        name: "broken".into(),
        nodes: vec![
            Node {
                id: 1,
                kind: NodeKind::ConstVec3([1.0, 0.0, 0.0]),
                inputs: vec![],
            },
            Node {
                id: 2,
                kind: NodeKind::SurfaceOutput,
                inputs: vec![None, link(1), None, None, None, None],
            },
        ],
    }
}

#[test]
fn matgraph_roundtrip_through_document() {
    let dir = temp("roundtrip");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("mat.matgraph");

    let mut doc = MaterialDocument::new("tint");
    doc.graph = valid_tint_graph();
    doc.path = Some(path.clone());
    doc.save().expect("save valid graph");

    let (loaded, _) = (
        MaterialDocument::open(&path, None).expect("open"),
        (),
    );
    assert_eq!(loaded.graph, doc.graph);
    assert!(!loaded.is_dirty());
}

#[test]
fn invalid_graph_save_err_and_disk_unchanged() {
    let dir = temp("invalid");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("mat.matgraph");

    let mut good = MaterialDocument::new("good");
    good.graph = valid_tint_graph();
    good.path = Some(path.clone());
    good.save().expect("seed file");
    let bytes_before = fs::read(&path).expect("read");

    let mut bad = MaterialDocument::open(&path, None).expect("open");
    bad.graph = broken_graph();
    let err = bad.save().expect_err("save must fail");
    assert!(matches!(err, MaterialError::SaveBlocked { .. }));
    let bytes_after = fs::read(&path).expect("read again");
    assert_eq!(bytes_before, bytes_after);
    assert!(bad.is_dirty());
}

#[test]
fn valid_graph_save_updates_bytes() {
    let dir = temp("update");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("mat.matgraph");

    let mut doc = MaterialDocument::new("tint");
    doc.graph = valid_tint_graph();
    doc.path = Some(path.clone());
    let compiled = doc.save().expect("save");
    assert!(!compiled.wgsl.is_empty());
    let first = fs::read(&path).expect("read");

    doc.graph.name = "tint_renamed".into();
    assert!(doc.is_dirty());
    doc.save().expect("save again");
    let second = fs::read(&path).expect("read");
    assert_ne!(first, second);
    assert!(!doc.is_dirty());
}

#[test]
fn compile_for_save_matches_document_gate() {
    let ok = compile_for_save(&valid_tint_graph());
    assert!(ok.is_ok());
    let bad = compile_for_save(&broken_graph());
    assert!(bad.is_err());
}