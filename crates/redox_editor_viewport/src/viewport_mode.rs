//! Viewport shading + quad-layout modes for the editor preview.
//!
//! Hotkeys (integrator / `redox_editor` main loop):
//! - **F1–F4** — [`ViewportShading`] (Lit / Unlit / Normals / Wireframe)
//! - **F5** — toggle [`ViewportLayout`] Single ↔ Quad
//! - **F6** — toggle [`ViewportDebugFlags::show_overdraw`]
//! - **F7** — toggle [`ViewportDebugFlags::show_aabbs`]
//!
//! Wireframe uses real LineList edge geometry ([`ViewportShading::is_wireframe`]).
//! Quad pick must use the same ortho extent as the quad pass ([`QUAD_ORTHO_HALF`]).

/// Ortho half-extent (world units) shared by [`crate::viewport::ViewportCubePass::render_quad`]
/// and [`crate::picker::pick_in_quad_pane`].
pub const QUAD_ORTHO_HALF: f32 = 24.0;

/// How the viewport shades the scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ViewportShading {
    /// Full lighting.
    #[default]
    Lit = 0,
    /// No lighting.
    Unlit = 1,
    /// Surface normals shown as colors.
    Normals = 2,
    /// Real edge/line geometry (not a fragment fresnel hack).
    Wireframe = 3,
}

impl ViewportShading {
    /// Shading mode bound to function key F1..F4.
    pub fn from_f_key(n: u8) -> Option<Self> {
        match n {
            1 => Some(Self::Lit),
            2 => Some(Self::Unlit),
            3 => Some(Self::Normals),
            4 => Some(Self::Wireframe),
            _ => None,
        }
    }

    /// Name shown in the UI.
    pub fn label(self) -> &'static str {
        match self {
            Self::Lit => "Lit (F1)",
            Self::Unlit => "Unlit (F2)",
            Self::Normals => "Normals (F3)",
            Self::Wireframe => "Wire (F4)",
        }
    }

    /// Numeric value passed to shaders.
    pub fn as_u32(self) -> u32 {
        self as u32
    }

    /// True when the cube pass should draw edge line lists instead of filled triangles.
    pub fn is_wireframe(self) -> bool {
        matches!(self, Self::Wireframe)
    }
}

/// One viewport or four.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ViewportLayout {
    /// A single view.
    #[default]
    Single,
    /// Four views: top, front, right and perspective.
    Quad,
}

impl ViewportLayout {
    /// Switches between single and quad.
    pub fn toggle(self) -> Self {
        match self {
            Self::Single => Self::Quad,
            Self::Quad => Self::Single,
        }
    }
}

/// Engine-style debug overlays. UI owns the flags; integrator applies them when drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ViewportDebugFlags {
    /// Highlight overdraw.
    pub show_overdraw: bool,
    /// Draw bounding boxes.
    pub show_aabbs: bool,
}

impl ViewportDebugFlags {
    /// F6 = overdraw, F7 = AABBs. Returns `true` if the key was handled.
    pub fn apply_f_key(&mut self, n: u8) -> bool {
        match n {
            6 => {
                self.show_overdraw = !self.show_overdraw;
                true
            }
            7 => {
                self.show_aabbs = !self.show_aabbs;
                true
            }
            _ => false,
        }
    }
}

/// Pane within the 2×2 quad viewport layout (matches `render_quad` scissor order).
///
/// ```text
/// Top        | Front
/// -----------+-------------
/// Right      | Perspective
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuadPane {
    /// Top-down pane.
    Top,
    /// Front pane.
    Front,
    /// Right-side pane.
    Right,
    /// Perspective pane.
    Perspective,
}

impl QuadPane {
    /// Name shown in the UI.
    pub fn label(self) -> &'static str {
        match self {
            Self::Top => "Top",
            Self::Front => "Front",
            Self::Right => "Right",
            Self::Perspective => "Perspective",
        }
    }
}
