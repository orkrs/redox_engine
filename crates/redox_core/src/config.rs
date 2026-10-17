//! Subsystem configuration and engine startup settings.

use serde::{Deserialize, Serialize};

/// Forward lighting path used by the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum LightingMode {
    /// Clustered forward shading (default).
    #[default]
    Clustered,
    /// ScatterLight — RedOx stochastic per-pixel point-light path (K=8).
    ScatterLight,
}

impl From<LightingMode> for redox_render::LightingMode {
    fn from(mode: LightingMode) -> Self {
        match mode {
            LightingMode::Clustered => Self::Clustered,
            LightingMode::ScatterLight => Self::ScatterLight,
        }
    }
}

/// Presentation mode for the window surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PresentMode {
    /// Vsync, letting the driver pick the best supported mode.
    AutoVsync,
    /// No vsync, letting the driver pick the best supported mode.
    AutoNoVsync,
    /// Classic vsync (always supported).
    Fifo,
    /// Vsync that tears when a frame is late.
    FifoRelaxed,
    /// No vsync; may tear.
    Immediate,
    /// Low-latency vsync that drops stale frames.
    Mailbox,
}

/// Configuration for engine startup and window creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineConfig {
    /// The title of the window.
    pub window_title: String,
    /// The width of the window in logical pixels.
    pub window_width: u32,
    /// The height of the window in logical pixels.
    pub window_height: u32,
    /// Whether the window should be created full-screen.
    pub fullscreen: bool,
    /// Whether VSync should be enabled.
    pub vsync: bool,
    /// Whether the window can be resized by the user.
    pub resizable: bool,
    /// Fixed timestep interval in seconds for `Stage::PhysicsSync`.
    pub fixed_timestep: f32,
    /// Internal render scale for TSR/TAAU (`< 1` = upscale, `1` = native).
    #[serde(default = "default_render_scale")]
    pub render_scale: f32,
    /// Sharp AA pipeline (MSAA + Smart Resolve + SMAA + RCAS). Default true.
    #[serde(default = "default_aa_sharp")]
    pub aa_sharp: bool,
    /// Lighting path: clustered forward or ScatterLight stochastic sampling.
    pub lighting_mode: LightingMode,
}

fn default_render_scale() -> f32 {
    1.0
}

fn default_aa_sharp() -> bool {
    true
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            window_title: "RedOx Engine".to_string(),
            window_width: 1280,
            window_height: 720,
            fullscreen: false,
            vsync: true,
            resizable: true,
            fixed_timestep: 1.0 / 60.0,
            render_scale: default_render_scale(),
            aa_sharp: default_aa_sharp(),
            lighting_mode: LightingMode::default(),
        }
    }
}

impl EngineConfig {
    /// Returns the active present mode based on `vsync` settings.
    pub fn present_mode(&self) -> PresentMode {
        if self.vsync {
            PresentMode::Fifo
        } else {
            PresentMode::AutoNoVsync
        }
    }

    /// Maps the config-level present mode to the `wgpu` equivalent.
    pub fn wgpu_present_mode(&self) -> wgpu::PresentMode {
        match self.present_mode() {
            PresentMode::AutoVsync => wgpu::PresentMode::AutoVsync,
            PresentMode::AutoNoVsync => wgpu::PresentMode::AutoNoVsync,
            PresentMode::Fifo => wgpu::PresentMode::Fifo,
            PresentMode::FifoRelaxed => wgpu::PresentMode::FifoRelaxed,
            PresentMode::Immediate => wgpu::PresentMode::Immediate,
            PresentMode::Mailbox => wgpu::PresentMode::Mailbox,
        }
    }

    /// Loads the engine configuration from an RON file.
    ///
    /// Missing fields fall back to [`EngineConfig::default`] values.
    pub fn load_from_file(path: &str) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => ron::from_str(&contents).unwrap_or_else(|e| {
                log::warn!("Failed to parse config {path}: {e}. Using defaults.");
                Self::default()
            }),
            Err(e) => {
                log::warn!("Failed to read config {path}: {e}. Using defaults.");
                Self::default()
            }
        }
    }

    /// Saves the configuration to an RON file (useful as a starting template).
    pub fn save_to_file(&self, path: &str) -> std::io::Result<()> {
        let mut out = String::new();
        out.push_str("// RedOx Engine configuration (Rusty Object Notation)\n");
        out.push_str(
            &ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
                .map_err(std::io::Error::other)?,
        );
        out.push('\n');
        std::fs::write(path, out)
    }
}
