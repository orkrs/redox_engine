//! Compilation diagnostics for the material editor (no egui).

use redox_matgraph::{GraphError, MaterialGraph};

use crate::material_document::compile_for_save;

/// One compiler message bound to a graph node when known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileMessage {
    /// Node the message refers to, if any.
    pub node: Option<u32>,
    /// Human-readable text (already includes node id when relevant).
    pub message: String,
}

impl CompileMessage {
    /// Builds from a [`GraphError`].
    pub fn from_graph_error(err: &GraphError) -> Self {
        Self {
            node: err.node,
            message: err.message.clone(),
        }
    }
}

/// Outcome of refreshing compile diagnostics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompileReport {
    /// Errors blocking save.
    pub errors: Vec<CompileMessage>,
    /// Whether the graph can be saved.
    pub save_ok: bool,
    /// Latest WGSL when save would succeed.
    pub wgsl: Option<String>,
}

impl CompileReport {
    /// Recomputes diagnostics for `graph`.
    pub fn refresh(graph: &MaterialGraph) -> Self {
        let validation: Vec<CompileMessage> = graph
            .validate()
            .iter()
            .map(CompileMessage::from_graph_error)
            .collect();
        if !validation.is_empty() {
            return Self {
                errors: validation,
                save_ok: false,
                wgsl: None,
            };
        }
        match compile_for_save(graph) {
            Ok(compiled) => Self {
                errors: Vec::new(),
                save_ok: true,
                wgsl: Some(compiled.wgsl),
            },
            Err(crate::material_document::MaterialError::SaveBlocked { errors, .. }) => Self {
                errors: errors
                    .iter()
                    .map(CompileMessage::from_graph_error)
                    .collect(),
                save_ok: false,
                wgsl: None,
            },
            Err(_) => Self {
                errors: vec![CompileMessage {
                    node: None,
                    message: "internal compile error".into(),
                }],
                save_ok: false,
                wgsl: None,
            },
        }
    }

    /// Index of the error to focus when the user picks message `index`.
    pub fn focus_node(&self, index: usize) -> Option<u32> {
        self.errors.get(index).and_then(|e| e.node)
    }
}
