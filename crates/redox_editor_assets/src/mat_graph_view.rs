//! UI-facing adapter over [`MaterialGraph`]: pin types, connections and undo via graph snapshots.

use std::collections::HashMap;

use redox_matgraph::graph::{
    MaterialGraph, NodeId, NodeKind, PortRef, Ty, arity, input_types, output_type,
};
use redox_matgraph::{GraphError, node_types};

/// 2D editor position of one node (not stored in `.matgraph` files).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeLayout {
    /// Canvas X.
    pub x: f32,
    /// Canvas Y.
    pub y: f32,
}

/// Result of trying to connect two pins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectError {
    /// Unknown node id.
    UnknownNode(NodeId),
    /// Port index out of range.
    BadPort {
        /// Node.
        node: NodeId,
        /// Port.
        port: usize,
    },
    /// Would create a cycle or fail typecheck.
    Rejected(Vec<GraphError>),
}

/// Undo history of whole-graph snapshots (one step per committed edit).
#[derive(Clone, Debug, Default)]
pub struct MatGraphUndo {
    past: Vec<MaterialGraph>,
    future: Vec<MaterialGraph>,
    limit: usize,
}

impl MatGraphUndo {
    /// Creates an undo stack with at most `limit` steps.
    pub fn new(limit: usize) -> Self {
        Self {
            past: Vec::new(),
            future: Vec::new(),
            limit: limit.max(1),
        }
    }

    /// Records `graph` before a mutation. Call once per user gesture on commit.
    pub fn push_snapshot(&mut self, graph: &MaterialGraph) {
        self.past.push(graph.clone());
        if self.past.len() > self.limit {
            self.past.remove(0);
        }
        self.future.clear();
    }

    /// Pops the last snapshot and returns it, pushing `current` onto redo.
    pub fn undo(&mut self, current: &MaterialGraph) -> Option<MaterialGraph> {
        let prev = self.past.pop()?;
        self.future.push(current.clone());
        Some(prev)
    }

    /// Redo after undo.
    pub fn redo(&mut self, current: &MaterialGraph) -> Option<MaterialGraph> {
        let next = self.future.pop()?;
        self.past.push(current.clone());
        Some(next)
    }

    /// Whether undo is available.
    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    /// Whether redo is available.
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }
}

/// Editable view of a material graph for the node canvas.
#[derive(Clone, Debug, PartialEq)]
pub struct MatGraphView {
    /// Authoring graph.
    pub graph: MaterialGraph,
    /// Layout positions keyed by node id.
    pub layout: HashMap<NodeId, NodeLayout>,
    /// Undo snapshots.
    pub undo: MatGraphUndo,
    /// Next id to assign when adding a node.
    next_id: NodeId,
}

impl MatGraphView {
    /// Wraps an existing graph and lays nodes on a simple grid.
    pub fn from_graph(graph: MaterialGraph) -> Self {
        let mut next_id = 1;
        for n in &graph.nodes {
            next_id = next_id.max(n.id.saturating_add(1));
        }
        let mut view = Self {
            graph,
            layout: HashMap::new(),
            undo: MatGraphUndo::new(64),
            next_id,
        };
        view.auto_layout();
        view
    }

    /// Output type of a node's port 0 after validation, when known.
    pub fn output_ty(&self, node: NodeId) -> Option<Ty> {
        let types = node_types(&self.graph).ok()?;
        types.get(&node).copied()
    }

    /// Expected input type of `port` on `node` (fixed ports only).
    pub fn input_ty(&self, node: NodeId, port: usize) -> Option<Option<Ty>> {
        let n = self.graph.nodes.iter().find(|n| n.id == node)?;
        input_types(&n.kind).get(port).copied()
    }

    /// Whether a link from `from` port 0 into `to` port `to_port` is allowed.
    pub fn can_connect(&self, from: NodeId, to: NodeId, to_port: usize) -> Result<(), ConnectError> {
        if !self.graph.nodes.iter().any(|n| n.id == from) {
            return Err(ConnectError::UnknownNode(from));
        }
        let Some(target) = self.graph.nodes.iter().find(|n| n.id == to) else {
            return Err(ConnectError::UnknownNode(to));
        };
        if to_port >= arity(&target.kind) {
            return Err(ConnectError::BadPort {
                node: to,
                port: to_port,
            });
        }
        let mut trial = self.graph.clone();
        let Some(node) = trial.nodes.iter_mut().find(|n| n.id == to) else {
            return Err(ConnectError::UnknownNode(to));
        };
        if node.inputs.len() <= to_port {
            node.inputs.resize(to_port + 1, None);
        }
        node.inputs[to_port] = Some(PortRef { node: from, port: 0 });
        let errors = trial.validate();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(ConnectError::Rejected(errors))
        }
    }

    /// Connects after validation; records undo when `commit` is true.
    pub fn connect(
        &mut self,
        from: NodeId,
        to: NodeId,
        to_port: usize,
        commit: bool,
    ) -> Result<(), ConnectError> {
        self.can_connect(from, to, to_port)?;
        if commit {
            self.undo.push_snapshot(&self.graph);
        }
        let Some(node) = self.graph.nodes.iter_mut().find(|n| n.id == to) else {
            return Err(ConnectError::UnknownNode(to));
        };
        if node.inputs.len() <= to_port {
            node.inputs.resize(to_port + 1, None);
        }
        node.inputs[to_port] = Some(PortRef { node: from, port: 0 });
        Ok(())
    }

    /// Adds a node and places it near `(x, y)`.
    pub fn add_node(&mut self, kind: NodeKind, x: f32, y: f32) -> NodeId {
        self.undo.push_snapshot(&self.graph);
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let inputs = vec![None; arity(&kind)];
        self.graph.nodes.push(redox_matgraph::graph::Node {
            id,
            kind,
            inputs,
        });
        self.layout.insert(id, NodeLayout { x, y });
        id
    }

    /// Removes a node and any links pointing to it.
    pub fn remove_node(&mut self, id: NodeId) {
        self.undo.push_snapshot(&self.graph);
        self.graph.nodes.retain(|n| n.id != id);
        for n in &mut self.graph.nodes {
            for slot in &mut n.inputs {
                if let Some(PortRef { node, .. }) = slot
                    && *node == id
                {
                    *slot = None;
                }
            }
        }
        self.layout.remove(&id);
    }

    /// Updates a scalar parameter default on a `Param` node.
    pub fn set_param_default(&mut self, id: NodeId, value: [f32; 4]) {
        self.undo.push_snapshot(&self.graph);
        let Some(node) = self.graph.nodes.iter_mut().find(|n| n.id == id) else {
            return;
        };
        if let NodeKind::Param { default, .. } = &mut node.kind {
            *default = value;
        }
    }

    /// Validates the graph and returns errors (each may name a node).
    pub fn validate(&self) -> Vec<GraphError> {
        self.graph.validate()
    }

    /// Assigns grid positions to nodes that have none.
    pub fn auto_layout(&mut self) {
        let mut sorted: Vec<NodeId> = self.graph.nodes.iter().map(|n| n.id).collect();
        sorted.sort_unstable();
        for (i, id) in sorted.iter().enumerate() {
            self.layout.entry(*id).or_insert(NodeLayout {
                x: 80.0 + (i % 4) as f32 * 180.0,
                y: 60.0 + (i / 4) as f32 * 110.0,
            });
        }
    }

    /// Inferred output type when the graph is valid (for pin labels).
    pub fn inferred_output(&self, node: NodeId) -> Option<Ty> {
        let n = self.graph.nodes.iter().find(|n| n.id == node)?;
        let mut ins = Vec::new();
        for slot in &n.inputs {
            if let Some(PortRef { node: src, .. }) = slot {
                ins.push(self.output_ty(*src).unwrap_or(Ty::F32));
            }
        }
        Some(output_type(&n.kind, &ins))
    }
}