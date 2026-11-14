//! Error model carries node ids (EDT-B5 step 3).

use redox_editor_assets::CompileReport;
use redox_matgraph::{MaterialGraph, Node, NodeKind, PortRef};

fn broken() -> MaterialGraph {
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
                inputs: vec![
                    None,
                    Some(PortRef { node: 1, port: 0 }),
                    None,
                    None,
                    None,
                    None,
                ],
            },
        ],
    }
}

#[test]
fn compile_report_error_carries_node_id() {
    let report = CompileReport::refresh(&broken());
    assert!(!report.save_ok);
    let err = report.errors.first().expect("one error");
    assert_eq!(err.node, Some(2));
    assert_eq!(report.focus_node(0), Some(2));
}
