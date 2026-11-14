//! Tests of `MatGraphView` (EDT-B5 step 2).

use redox_editor_assets::{ConnectError, MatGraphView};
use redox_matgraph::{MaterialGraph, Node, NodeKind, PortRef};

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

#[test]
fn incompatible_connect_is_rejected() {
    let mut view = MatGraphView::from_graph(tint_graph());
    let scalar = view.add_node(NodeKind::ConstF32(0.5), 10.0, 10.0);
    let err = view
        .connect(scalar, 2, 0, false)
        .expect_err("vec3 base_color rejects f32");
    assert!(matches!(err, ConnectError::Rejected(_)));
    let surface = view
        .graph
        .nodes
        .iter()
        .find(|n| n.id == 2)
        .expect("surface node");
    assert_eq!(surface.inputs[0], link(1), "graph must stay unchanged");
}

#[test]
fn undo_restores_graph_after_add_node() {
    let mut view = MatGraphView::from_graph(tint_graph());
    let before = view.graph.clone();
    let _ = view.add_node(NodeKind::Time, 0.0, 0.0);
    assert_ne!(view.graph, before);
    assert!(view.undo.can_undo());
    let restored = view.undo.undo(&view.graph).expect("undo");
    view.graph = restored;
    assert_eq!(view.graph, before);
}
