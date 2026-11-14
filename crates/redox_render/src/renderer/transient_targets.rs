//! Rebuilds the transient texture pool and binds logical views on the renderer.

use super::*;
use crate::frame_graph::{self, FrameFeatures};
use crate::transient_pool::{PoolKey, TransientPool};

impl Renderer {
    /// Features that control optional passes in the frame graph declaration.
    pub fn frame_features(&self) -> FrameFeatures {
        FrameFeatures {
            fog: self.post_fx_settings.fog.is_some(),
        }
    }

    /// Recompiles the frame graph and (re)allocates pooled transients for the current size.
    pub(crate) fn rebuild_transient_pool(&mut self) {
        let features = self.frame_features();
        let graph = frame_graph::frame_graph_with_features(
            self.internal_width,
            self.internal_height,
            features,
        )
        .compile()
        .expect("frame graph is valid");
        let key = PoolKey {
            width: self.internal_width,
            height: self.internal_height,
            features,
        };
        self.transient_pool
            .rebuild(&self.device, &graph, key);
        self.bind_transient_views();
        self.rebind_pool_dependent_resources();
        self.compiled_frame_graph = graph.clone();
    }

    fn rebind_pool_dependent_resources(&mut self) {
        self.velocity_bg = self
            .velocity_pass
            .create_bind_group(&self.device, &self.depth_view);
        self.rebuild_bloom_bind_groups();
    }

    fn bind_transient_views(&mut self) {
        if let Some(tex) = self.transient_pool.texture("depth") {
            self.depth_texture = tex.clone();
        }
        if let Some(view) = self.transient_pool.view("depth") {
            self.depth_view = view.clone();
        }
        if let Some(tex) = self.transient_pool.texture("hdr_color") {
            self.hdr_texture = tex.clone();
        }
        if let Some(view) = self.transient_pool.view("hdr_color") {
            self.hdr_view = view.clone();
        }
        if let Some(tex) = self.transient_pool.texture("normals") {
            self.normal_texture = tex.clone();
        }
        if let Some(view) = self.transient_pool.view("normals") {
            self.normal_view = view.clone();
        }
        if let Some(tex) = self.transient_pool.texture("ao") {
            self.ssao_blurred_texture = tex.clone();
        }
        if let Some(view) = self.transient_pool.view("ao") {
            self.ssao_blurred_view = view.clone();
        }
    }

    /// Frame graph report at the current internal resolution (includes aliased savings).
    pub fn frame_graph_report_with_pool(&self) -> String {
        let features = self.frame_features();
        let mut out = frame_graph::frame_graph_report_with_features(
            self.internal_width,
            self.internal_height,
            features,
        );
        let saved = self.transient_pool.aliased_saved_bytes(&self.compiled_frame_graph);
        let _ = std::fmt::Write::write_fmt(
            &mut out,
            format_args!("pool_aliased_saved_bytes: {saved}\n"),
        );
        out
    }
}