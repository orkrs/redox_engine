//! Subsystem configuration and engine startup settings.

/// Presentation mode for the window surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresentMode {
    AutoVsync,
    AutoNoVsync,
    Fifo,
    FifoRelaxed,
    Immediate,
    Mailbox,
}

/// Configuration for engine startup and window creation.
#[derive(Debug, Clone)]
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

    /// Loads the engine configuration from a file.
    ///
    /// Currently unimplemented.
    pub fn load_from_file(_path: &str) -> Self {
        unimplemented!(
            "Loading config from file is not yet supported. Use EngineConfig::default() instead."
        );
    }
}
