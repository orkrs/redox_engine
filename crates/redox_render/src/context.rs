//! Thin facade over [`crate::renderer::Renderer`] for backward-compatible examples.

use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use winit::window::Window;

use crate::frame::FrameView;
use crate::renderer::Renderer;
pub use crate::renderer::ShaderDebugVizMode;
use crate::systems::RenderObject;
use crate::virtual_geometry::runtime::VGConfig;

/// Backward-compatible rendering facade. Delegates to [`Renderer`].
pub struct RenderContext(Renderer);

impl Deref for RenderContext {
    type Target = Renderer;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for RenderContext {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl RenderContext {
    pub async fn new(window: Arc<Window>, present_mode: wgpu::PresentMode) -> Self {
        Self(Renderer::new(window, present_mode).await)
    }

    /// Preferred entry point: render one frame from a [`FrameView`].
    pub fn render_frame_view(
        &mut self,
        fv: &FrameView<'_>,
        surface_view: &wgpu::TextureView,
    ) -> Result<(), crate::surface::SurfaceError> {
        self.0.render_frame_view(fv, surface_view)
    }

    /// Legacy single-call render (acquires surface texture internally).
    pub fn render_frame(
        &mut self,
        objects: &[RenderObject],
    ) -> Result<(), crate::surface::SurfaceError> {
        self.0.render_frame(objects)
    }

    pub fn init_virtual_geometry(&mut self, config: VGConfig) {
        self.0.init_virtual_geometry(config);
    }
}

pub use crate::registry::GpuMesh;
