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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_error_messages_name_the_failing_step() {
        assert!(
            RendererError::NoAdapter
                .to_string()
                .to_lowercase()
                .contains("adapter")
        );
        assert!(
            RendererError::Surface("x".into())
                .to_string()
                .to_lowercase()
                .contains("surface")
        );
        assert!(
            RendererError::Device("lost".into())
                .to_string()
                .contains("lost")
        );
    }

    #[test]
    fn renderer_error_can_cross_threads_and_be_boxed() {
        fn is_error<T: std::error::Error + Send + Sync + 'static>() {}
        is_error::<RendererError>();
        let boxed: Box<dyn std::error::Error> = Box::new(RendererError::NoAdapter);
        assert!(!boxed.to_string().is_empty());
    }
}
