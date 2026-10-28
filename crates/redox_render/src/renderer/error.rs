//! Errors returned when a [`Renderer`](super::Renderer) cannot be created.

/// Why a [`Renderer`](super::Renderer) could not be created.
#[derive(Debug)]
pub enum RendererError {
    /// The window surface could not be created.
    Surface(String),
    /// No GPU adapter supports the surface and the required features.
    NoAdapter,
    /// The adapter refused the requested device.
    Device(String),
}

impl std::fmt::Display for RendererError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Surface(reason) => write!(f, "failed to create surface: {reason}"),
            Self::NoAdapter => write!(f, "no suitable GPU adapter found"),
            Self::Device(reason) => write!(f, "failed to create device: {reason}"),
        }
    }
}

impl std::error::Error for RendererError {}
