//! Debug-build check of the frame graph against what the passes touch (RND-09 step 2).
//!
//! The renderer reports each texture a pass hands to the GPU through [`FrameState::touch`]; at the
//! end of the frame the log is compared with the declaration by `CompiledGraph::verify_usage`.
//! The whole module exists only in debug builds: in release `FrameState::touch` is empty.

use crate::render_graph::{AccessMismatch, CompiledGraph, PassId, UsageLog};

/// Usage log of one frame, checked against the frame graph of the same target size.
pub(super) struct FrameUsage {
    graph: Option<CompiledGraph>,
    log: UsageLog,
    pass: Option<PassId>,
}

impl FrameUsage {
    /// Starts a log for a frame of `width` x `height`. The graph structure does not depend on
    /// the size, but it is compiled for the actual target so the declaration matches the frame.
    pub(super) fn for_size(width: u32, height: u32) -> Self {
        let graph = crate::frame_graph::frame_graph(width, height).compile().ok();
        Self {
            graph,
            log: UsageLog::new(),
            pass: None,
        }
    }

    /// Makes `name` the pass that the following touches belong to.
    pub(super) fn begin_pass(&mut self, name: &str) {
        self.pass = self.graph.as_ref().and_then(|g| g.pass_by_name(name));
    }

    /// Records that the current pass uses the texture `name` of the graph.
    pub(super) fn touch(&mut self, texture: &str) {
        let (Some(graph), Some(pass)) = (self.graph.as_ref(), self.pass) else {
            return;
        };
        if let Some(tex) = graph.texture_by_name(texture) {
            self.log.record_texture(pass, tex);
        }
    }

    /// Declared-versus-used differences of the finished frame (empty when the graph did not
    /// compile, which the graph tests rule out).
    pub(super) fn mismatches(&self) -> Vec<AccessMismatch> {
        self.graph
            .as_ref()
            .map(|g| g.verify_usage(&self.log))
            .unwrap_or_default()
    }
}
