//! Per-frame render inputs assembled by the game / app layer.

use crate::camera::Camera;
use crate::light::LightScene;
use crate::resource::ibl::IBLResource;
use crate::systems::RenderObject;

/// Everything the renderer needs to draw one frame besides GPU state.
///
/// Pass order is fixed inside [`crate::renderer::Renderer::render_frame`]:
/// shadows → normals → SSAO → skybox → opaque PBR → transparent → VG → TAA → tonemap.
pub struct FrameView<'a> {
    /// Camera that renders the frame.
    pub camera: &'a Camera,
    /// Objects to draw.
    pub objects: &'a [RenderObject],
    /// Lights of the frame.
    pub lights: &'a LightScene,
    /// Image-based lighting environment, if any.
    pub environment: Option<&'a IBLResource>,
    /// Stable id of every object (same length as `objects`), if the caller has them.
    /// With ids the renderer can tell which objects moved between frames even when the list
    /// changes; without them objects are matched by their position in the list.
    pub object_ids: Option<&'a [u64]>,
}

impl<'a> FrameView<'a> {
    /// Describes a frame without an environment.
    pub fn new(camera: &'a Camera, objects: &'a [RenderObject], lights: &'a LightScene) -> Self {
        Self {
            camera,
            objects,
            lights,
            environment: None,
            object_ids: None,
        }
    }

    /// Supplies a stable id per object (see [`Self::object_ids`]).
    pub fn with_object_ids(mut self, ids: &'a [u64]) -> Self {
        self.object_ids = Some(ids);
        self
    }

    /// Adds an image-based lighting environment.
    pub fn with_environment(mut self, env: &'a IBLResource) -> Self {
        self.environment = Some(env);
        self
    }
}
