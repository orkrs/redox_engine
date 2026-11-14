//! Material graph drawing on the shared node canvas (EDT-B5 step 3).

use redox_editor_assets::{CompileReport, MatGraphView, NodeLayout};
use redox_matgraph::NodeId;
use redox_matgraph::graph::{NodeKind, PortRef};

use crate::node_canvas::{Edge, GraphMode, Node, NodeCanvas, NodeGraph};

/// Builds the canvas topology from `view`.
pub fn graph_from_view(view: &MatGraphView) -> NodeGraph {
    let mut nodes = Vec::with_capacity(view.graph.nodes.len());
    for n in &view.graph.nodes {
        let pos = view
            .layout
            .get(&n.id)
            .map(|l| egui::pos2(l.x, l.y))
            .unwrap_or(egui::pos2(80.0, 80.0));
        let title = node_label(&n.kind, n.id);
        nodes.push(Node {
            id: n.id as usize,
            kind: title,
            pos,
        });
    }
    let mut id_to_idx = std::collections::HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        id_to_idx.insert(n.id, i);
    }
    let mut edges = Vec::new();
    for n in &view.graph.nodes {
        let Some(&from_idx) = id_to_idx.get(&(n.id as usize)) else {
            continue;
        };
        for (port, input) in n.inputs.iter().enumerate() {
            let Some(PortRef { node, .. }) = input else {
                continue;
            };
            let Some(&to_idx) = id_to_idx.get(&(*node as usize)) else {
                continue;
            };
            edges.push(Edge {
                from: to_idx,
                to: from_idx,
                label: format!("p{port}"),
            });
        }
    }
    NodeGraph { nodes, edges }
}

fn node_label(kind: &NodeKind, id: NodeId) -> String {
    let base = match kind {
        NodeKind::Param { name, .. } => format!("Param:{name}"),
        NodeKind::ConstF32(v) => format!("{v:.3}"),
        NodeKind::ConstVec3(v) => format!("vec3({:.2},{:.2},{:.2})", v[0], v[1], v[2]),
        NodeKind::SurfaceOutput => "Surface".into(),
        NodeKind::Custom { name, .. } => format!("Custom:{name}"),
        other => format!("{other:?}"),
    };
    let mut label = format!("{base} #{id}");
    label.truncate(56);
    label
}

/// Maps compile errors to canvas node indices for highlighting.
pub fn focus_from_report(report: &CompileReport, focused: Option<NodeId>) -> Option<usize> {
    focused
        .or_else(|| report.errors.first().and_then(|e| e.node))
        .map(|id| id as usize)
}

/// Material-mode canvas: sync layout, drag nodes, paint.
pub fn draw_material(
    canvas: &mut NodeCanvas,
    ui: &mut egui::Ui,
    rect: egui::Rect,
    response: &egui::Response,
    view: &mut MatGraphView,
) {
    canvas.graph = graph_from_view(view);
    for n in &mut canvas.graph.nodes {
        if let Some(layout) = view.layout.get(&(n.id as u32)) {
            n.pos = egui::pos2(layout.x, layout.y);
        }
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, egui::Color32::from_gray(28));
    if response.dragged()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let rel = pos - rect.min.to_vec2();
        if let Some(n) =
            canvas.graph.nodes.iter_mut().find(|n| {
                egui::Rect::from_center_size(n.pos, egui::vec2(130.0, 44.0)).contains(rel)
            })
        {
            n.pos += response.drag_delta();
            view.layout.insert(
                n.id as u32,
                NodeLayout {
                    x: n.pos.x,
                    y: n.pos.y,
                },
            );
        }
    }
    paint_graph(
        &painter,
        rect,
        &canvas.graph,
        canvas.active_id,
        GraphMode::Material,
    );
}

fn paint_graph(
    painter: &egui::Painter,
    rect: egui::Rect,
    graph: &NodeGraph,
    active_id: Option<usize>,
    mode: GraphMode,
) {
    for edge in &graph.edges {
        if let (Some(a), Some(b)) = (graph.nodes.get(edge.from), graph.nodes.get(edge.to)) {
            let p0 = a.pos + rect.min.to_vec2();
            let p1 = b.pos + rect.min.to_vec2();
            painter.line_segment(
                [p0, p1],
                egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 180, 255)),
            );
        }
    }
    for node in &graph.nodes {
        let r =
            egui::Rect::from_center_size(node.pos + rect.min.to_vec2(), egui::vec2(130.0, 44.0));
        let active = active_id == Some(node.id);
        let fill = if active {
            egui::Color32::from_rgb(140, 60, 60)
        } else if mode == GraphMode::Material {
            egui::Color32::from_rgb(70, 50, 100)
        } else {
            egui::Color32::from_gray(48)
        };
        painter.rect_filled(r, 6.0, fill);
        if active {
            painter.rect_stroke(
                r,
                6.0,
                egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 120, 80)),
                egui::StrokeKind::Inside,
            );
        }
        painter.text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            &node.kind,
            egui::FontId::proportional(12.0),
            egui::Color32::WHITE,
        );
    }
}
