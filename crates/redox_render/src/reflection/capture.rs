//! Capture of a reflection probe: renders the six cube faces around the probe centre, prefilters
//! them and copies the result into the probe's slot of the cube array.

use redox_math::{Vec3, look_at, perspective};

use super::gpu::ProbesGpu;
use super::probe::ReflectionProbe;
use super::set::{PROBE_FACE_SIZE, PROBE_MIP_LEVELS};
use crate::camera::{Camera, CameraUniform};
use crate::frame::FrameView;
use crate::light::LightScene;
use crate::renderer::Renderer;
use crate::surface::SurfaceError;
use crate::systems::RenderObject;

/// Cube faces per probe.
const FACES: u32 = 6;
/// Near plane of the capture cameras.
const CAPTURE_NEAR: f32 = 0.05;
/// Far plane of the capture cameras.
const CAPTURE_FAR: f32 = 1000.0;

/// Why a probe capture failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeCaptureError {
    /// The slot index has no active probe.
    SlotEmpty(usize),
    /// Rendering one of the cube faces failed.
    Render(SurfaceError),
}

impl std::fmt::Display for ProbeCaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SlotEmpty(slot) => write!(f, "reflection probe slot {slot} is empty"),
            Self::Render(error) => write!(f, "reflection probe capture failed: {error:?}"),
        }
    }
}

impl std::error::Error for ProbeCaptureError {}

impl From<SurfaceError> for ProbeCaptureError {
    fn from(error: SurfaceError) -> Self {
        Self::Render(error)
    }
}

/// Forward and up vectors of the six cube faces, in `+X -X +Y -Y +Z -Z` order.
fn face_axes() -> [(Vec3, Vec3); 6] {
    [
        (Vec3::X, Vec3::Y),
        (Vec3::NEG_X, Vec3::Y),
        (Vec3::Y, Vec3::NEG_Z),
        (Vec3::NEG_Y, Vec3::Z),
        (Vec3::Z, Vec3::Y),
        (Vec3::NEG_Z, Vec3::Y),
    ]
}

/// Camera uniform that looks from `position` along `forward` with the given `up`.
///
/// Written out directly because `CameraUniform::update` always uses +Y as up, which is
/// degenerate for the +Y and -Y faces.
fn face_uniform(
    base: &CameraUniform,
    camera: &Camera,
    position: Vec3,
    forward: Vec3,
    up: Vec3,
) -> CameraUniform {
    let view = look_at(position, position + forward, up);
    let proj = perspective(camera.fov_y, camera.aspect_ratio, camera.near, camera.far);
    let view_proj = proj * view;
    CameraUniform {
        view: view.to_cols_array_2d(),
        proj: proj.to_cols_array_2d(),
        inv_view: view.inverse().to_cols_array_2d(),
        inv_proj: proj.inverse().to_cols_array_2d(),
        view_proj: view_proj.to_cols_array_2d(),
        camera_pos: [position.x, position.y, position.z, 1.0],
        ..*base
    }
}

impl Renderer {
    /// Captures the scene around the probe in `slot` into the probe array.
    ///
    /// Renders the six faces (sky and objects, post effects as the renderer applies them to
    /// the frame), prefilters the cube and copies it into the slot. Call again after the
    /// scene changes. The light uniform and camera uniform are restored afterwards.
    ///
    /// Known limitation: the faces go through the normal tone-mapped path, so the capture is
    /// LDR in the surface format, not HDR; very bright sky or lights clip in reflections.
    /// Probes are not sampled while capturing.
    pub fn capture_reflection_probe(
        &mut self,
        slot: usize,
        objects: &[RenderObject],
        lights: &LightScene,
    ) -> Result<(), ProbeCaptureError> {
        let probe = *self
            .reflection_probes
            .probes()
            .get(slot)
            .ok_or(ProbeCaptureError::SlotEmpty(slot))?;
        let target = self.create_probe_capture_target();
        let saved_camera = self.camera_uniform;
        let saved_light = self.pbr_pass_light_uniform();
        let camera = Camera::new(std::f32::consts::FRAC_PI_2, 1.0, CAPTURE_NEAR, CAPTURE_FAR);

        // Captures must not sample probes (recursion): the table is empty while the faces render.
        self.reflection_probes
            .write_table(&self.queue, &ProbesGpu::default());
        let result = self.render_probe_faces(&target, &camera, lights, objects, &probe);
        self.reflection_probes.upload(&self.queue);
        self.camera_uniform = saved_camera;
        self.update_light_buffer(&saved_light);
        result?;

        let cube = target.create_view(&wgpu::TextureViewDescriptor {
            label: Some("reflection_probe_capture_cube"),
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        let (prefiltered, _view) =
            self.ibl_processor
                .generate_prefiltered_map(&self.device, &self.queue, &cube);
        self.copy_prefiltered_into_slot(&prefiltered, slot);
        Ok(())
    }

    /// Cube texture the six faces are rendered into, in the surface format.
    fn create_probe_capture_target(&self) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("reflection_probe_capture"),
            size: wgpu::Extent3d {
                width: PROBE_FACE_SIZE,
                height: PROBE_FACE_SIZE,
                depth_or_array_layers: FACES,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.surface_format(),
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    }

    fn render_probe_faces(
        &mut self,
        target: &wgpu::Texture,
        camera: &Camera,
        lights: &LightScene,
        objects: &[RenderObject],
        probe: &ReflectionProbe,
    ) -> Result<(), ProbeCaptureError> {
        let base = self.camera_uniform;
        for (face, (forward, up)) in face_axes().into_iter().enumerate() {
            self.camera_uniform = face_uniform(&base, camera, probe.center, forward, up);
            let face_view = target.create_view(&wgpu::TextureViewDescriptor {
                label: Some("reflection_probe_face"),
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: face as u32,
                array_layer_count: Some(1),
                ..Default::default()
            });
            let frame = FrameView::new(camera, objects, lights);
            self.render_frame_view(&frame, &face_view)?;
        }
        Ok(())
    }

    /// Copies all mips and faces of a prefiltered cube into the probe array slot.
    fn copy_prefiltered_into_slot(&self, prefiltered: &wgpu::Texture, slot: usize) {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("reflection_probe_copy"),
            });
        let base_layer = slot as u32 * FACES;
        for mip in 0..PROBE_MIP_LEVELS {
            let size = (PROBE_FACE_SIZE >> mip).max(1);
            for face in 0..FACES {
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: prefiltered,
                        mip_level: mip,
                        origin: wgpu::Origin3d { x: 0, y: 0, z: face },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: self.reflection_probes.texture(),
                        mip_level: mip,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: base_layer + face,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: size,
                        height: size,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
        self.queue.submit(Some(encoder.finish()));
    }
}
