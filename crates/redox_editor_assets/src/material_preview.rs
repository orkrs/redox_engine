//! Material preview settings for the editor (mesh shape and lighting). No GPU here.

/// Shape drawn in the material preview viewport.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PreviewMesh {
    /// Unit sphere (default).
    #[default]
    Sphere,
    /// Axis-aligned cube.
    Cube,
    /// Ground plane.
    Plane,
}

/// Lighting used when B3 IBL is not wired in the editor host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PreviewLighting {
    /// Single directional + ambient (fallback when B3 is absent).
    #[default]
    Simple,
    /// Use the viewport IBL probe when the host provides one (B3).
    ViewportIbl,
}

/// User-tunable preview options stored with the material editor session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MaterialPreviewSettings {
    /// Preview mesh.
    pub mesh: PreviewMesh,
    /// Lighting mode.
    pub lighting: PreviewLighting,
}

/// Simple directional light used by the fallback preview rig.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimplePreviewLight {
    /// Normalised direction (from surface toward the light).
    pub direction: [f32; 3],
    /// Linear colour/intensity.
    pub colour: [f32; 3],
    /// Ambient term added before the directional term.
    pub ambient: [f32; 3],
}

impl SimplePreviewLight {
    /// Default studio rig when B3 lighting is unavailable.
    pub fn studio_fallback() -> Self {
        Self {
            direction: [0.3, -0.7, -0.5],
            colour: [1.0, 1.0, 1.0],
            ambient: [0.35, 0.35, 0.38],
        }
    }
}