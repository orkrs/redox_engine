//! Maps editor [`ViewportShading`] to engine debug modes without fake surrogates.

use redox_render::Renderer;
use redox_render::context::ShaderDebugVizMode;

use crate::viewport_mode::ViewportShading;

/// Availability of a shading mode on the real engine path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShadingAvailability {
    /// Mode is wired to the renderer.
    Available,
    /// Engine has no implementation; viewport stays on Lit and UI shows the reason.
    Unavailable { reason: &'static str },
}

/// Effective GPU shading + honesty metadata for the overlay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewportShadingPlan {
    /// What the user asked for.
    pub requested: ViewportShading,
    /// What the GPU actually runs (Lit when unavailable).
    pub effective: ViewportShading,
    pub status: ShadingAvailability,
    /// Engine debug uniform when Lit-equivalent path is active.
    pub debug_mode: ShaderDebugVizMode,
    /// When true, integrator should draw triangle-edge lines from real meshes.
    pub wireframe_overlay: bool,
}

impl ViewportShadingPlan {
    /// Resolves `requested` against current engine capabilities (read from `redox_render`).
    pub fn resolve(requested: ViewportShading) -> Self {
        match requested {
            ViewportShading::Lit => Self {
                requested,
                effective: ViewportShading::Lit,
                status: ShadingAvailability::Available,
                debug_mode: ShaderDebugVizMode::None,
                wireframe_overlay: false,
            },
            ViewportShading::Wireframe => Self {
                requested,
                effective: ViewportShading::Lit,
                status: ShadingAvailability::Available,
                debug_mode: ShaderDebugVizMode::None,
                wireframe_overlay: true,
            },
            ViewportShading::Unlit => Self {
                requested,
                effective: ViewportShading::Lit,
                status: ShadingAvailability::Unavailable {
                    reason: "engine has no Unlit forward override (no LightingMode::Unlit)",
                },
                debug_mode: ShaderDebugVizMode::None,
                wireframe_overlay: false,
            },
            ViewportShading::Normals => Self {
                requested,
                effective: ViewportShading::Lit,
                status: ShadingAvailability::Unavailable {
                    reason: "ShaderDebugVizMode has no world-normal view (only cluster/shadow GI)",
                },
                debug_mode: ShaderDebugVizMode::None,
                wireframe_overlay: false,
            },
            ViewportShading::Roughness => Self {
                requested,
                effective: ViewportShading::Lit,
                status: ShadingAvailability::Unavailable {
                    reason: "no roughness debug view in pbr.wgsl",
                },
                debug_mode: ShaderDebugVizMode::None,
                wireframe_overlay: false,
            },
            ViewportShading::Depth => Self {
                requested,
                effective: ViewportShading::Lit,
                status: ShadingAvailability::Unavailable {
                    reason: "depth view requires prepass export API (not exposed on Renderer)",
                },
                debug_mode: ShaderDebugVizMode::None,
                wireframe_overlay: false,
            },
        }
    }

    /// Human label for toolbar overlay.
    pub fn overlay_label(&self) -> String {
        match &self.status {
            ShadingAvailability::Available if self.requested == ViewportShading::Wireframe => {
                "Shading: Wireframe (mesh edges)".into()
            }
            ShadingAvailability::Available => self.requested.label().into(),
            ShadingAvailability::Unavailable { reason } => {
                format!(
                    "Shading unavailable: {} — {}",
                    self.requested.label(),
                    reason
                )
            }
        }
    }
}

/// Applies the plan to the renderer (debug uniform only — no cube hacks).
pub fn apply_shading_plan(renderer: &mut Renderer, plan: &ViewportShadingPlan) {
    renderer.update_shader_debug_buffer(plan.debug_mode);
    let _ = plan.effective;
}
