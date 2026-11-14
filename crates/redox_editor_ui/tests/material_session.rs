//! Material editor session (EDT-B5 steps 3 and 5).

use redox_editor_ui::MaterialEditorSession;
use redox_matgraph::{MaterialGraph, Node, NodeKind, PortRef, Ty};
use std::fs;
use std::path::PathBuf;

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("edt_b5_sess_{name}_{}", std::process::id()))
}

fn link(id: u32) -> Option<PortRef> {
    Some(PortRef { node: id, port: 0 })
}

fn tint_graph() -> MaterialGraph {
    MaterialGraph {
        version: 1,
        name: "tint".into(),
        nodes: vec![
            Node {
                id: 1,
                kind: NodeKind::Param {
                    name: "tint".into(),
                    ty: Ty::Vec3,
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

#[test]
fn param_change_marks_dirty_then_save_ok() {
    let dir = temp("param");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("tint.matgraph");
    fs::write(&path, tint_graph().to_ron()).expect("seed");

    let mut session = MaterialEditorSession::open(&path, None).expect("open");
    assert!(!session.document.is_dirty());
    session.set_param(1, [0.1, 0.9, 0.2, 0.0]);
    assert!(session.document.is_dirty());
    session.save().expect("save after param tweak");
    assert!(!session.document.is_dirty());
    assert!(session.preview_revision > 0);
}