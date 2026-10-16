//! Swap-chain acquisition with a stable error type.
//!
//! wgpu 29 reports the outcome of `get_current_texture` as the enum
//! `CurrentSurfaceTexture`; the renderer API keeps a `Result` shape via [`SurfaceError`].

/// Why a swap-chain texture could not be acquired for this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SurfaceError {
    /// The surface was lost and must be recreated.
    #[error("surface lost")]
    Lost,
    /// The surface configuration is outdated: reconfigure and retry.
    #[error("surface outdated")]
    Outdated,
    /// Acquiring the frame timed out; skip this frame.
    #[error("surface timeout")]
    Timeout,
    /// The window is occluded; skip this frame.
    #[error("surface occluded")]
    Occluded,
    /// A validation error occurred while acquiring the frame.
    #[error("surface validation error")]
    Validation,
}

/// Acquires the next swap-chain texture. A suboptimal texture is still returned.
pub fn acquire_surface_texture(
    surface: &wgpu::Surface<'_>,
) -> Result<wgpu::SurfaceTexture, SurfaceError> {
    match surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
            Ok(t)
        }
        wgpu::CurrentSurfaceTexture::Timeout => Err(SurfaceError::Timeout),
        wgpu::CurrentSurfaceTexture::Occluded => Err(SurfaceError::Occluded),
        wgpu::CurrentSurfaceTexture::Outdated => Err(SurfaceError::Outdated),
        wgpu::CurrentSurfaceTexture::Lost => Err(SurfaceError::Lost),
        _ => Err(SurfaceError::Validation),
    }
}
