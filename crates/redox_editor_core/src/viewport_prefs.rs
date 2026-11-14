//! Serializable viewport toggles (sky, GI, fog) for editor session.

use serde::{Deserialize, Serialize};

/// Debug / lighting toggles applied on the engine [`Renderer`] each frame.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewportRuntimeSettings {
    /// Use procedural sky + IBL rebake.
    pub procedural_sky: bool,
    /// Local GI probes in the editor view.
    pub gi_enabled: bool,
    /// Analytic / froxel fog (requires fog settings + TAA in engine).
    pub fog_enabled: bool,
    /// Request SDF/GI debug visualization (engine may mark unavailable).
    pub sdf_debug: bool,
    /// Draw FPS / CPU / GPU strip on the viewport.
    pub show_stats_overlay: bool,
    /// Virtual geometry draw path (default off per spec).
    pub vg_enabled: bool,
}

impl Default for ViewportRuntimeSettings {
    fn default() -> Self {
        Self {
            procedural_sky: false,
            gi_enabled: false,
            fog_enabled: false,
            sdf_debug: false,
            show_stats_overlay: true,
            vg_enabled: false,
        }
    }
}

/// Result of applying [`ViewportRuntimeSettings`] to the engine.
#[derive(Clone, Debug, Default)]
pub struct ViewportToggleStatus {
    /// Overlay text when procedural sky / IBL could not fully apply.
    pub procedural_sky: &'static str,
    /// Overlay when GI is on but the editor did not seed SDF.
    pub gi: &'static str,
    /// Overlay when fog could not attach to the frame graph.
    pub fog: &'static str,
    /// Overlay for SDF debug surrogate.
    pub sdf_debug: &'static str,
}

impl ViewportToggleStatus {
    /// Message shown when a toggle could not be applied.
    pub fn overlay_lines(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if !self.procedural_sky.is_empty() {
            out.push(self.procedural_sky);
        }
        if !self.gi.is_empty() {
            out.push(self.gi);
        }
        if !self.fog.is_empty() {
            out.push(self.fog);
        }
        if !self.sdf_debug.is_empty() {
            out.push(self.sdf_debug);
        }
        out
    }
}
