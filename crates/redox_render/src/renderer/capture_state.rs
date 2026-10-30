//! Renderer state that a reflection probe capture would otherwise leave behind.
//!
//! A capture renders six faces through the ordinary frame path. That path advances the TAA
//! frame counter and ping-pong index, overwrites the TAA history and the previous-frame depth,
//! overwrites the VSM screen-shadow history, and feeds the VSM clipmap and page feedback.
//! [`Renderer::begin_probe_capture`] saves all of it and [`Renderer::end_probe_capture`] puts it
//! back, so the frame after a capture matches the frame of a run without one.
//!
//! While the capture is active, `probe_capture_active` stops the parts that cannot be saved
//! cheaply (VSM clipmap and page feedback, object motion history) from being touched at all.

use super::*;

/// A GPU texture that carries state from one frame to the next.
#[derive(Clone, Copy)]
enum TemporalTexture {
    TaaHistory(usize),
    PrevDepth,
    VsmShadowHistory,
}

impl TemporalTexture {
    const ALL: [Self; 4] = [
        Self::TaaHistory(0),
        Self::TaaHistory(1),
        Self::PrevDepth,
        Self::VsmShadowHistory,
    ];

    fn texture(self, renderer: &Renderer) -> &wgpu::Texture {
        match self {
            Self::TaaHistory(index) => &renderer.taa_history_tex[index],
            Self::PrevDepth => &renderer.prev_depth_texture,
            Self::VsmShadowHistory => &renderer.vsm_shadow_history_texture,
        }
    }

    fn aspect(self) -> wgpu::TextureAspect {
        match self {
            Self::PrevDepth => wgpu::TextureAspect::DepthOnly,
            Self::TaaHistory(_) | Self::VsmShadowHistory => wgpu::TextureAspect::All,
        }
    }
}

/// Everything a probe capture changes, saved before its faces render.
pub(crate) struct ProbeCaptureState {
    camera_uniform: CameraUniform,
    light_uniform: LightUniform,
    camera_near: f32,
    camera_far: f32,
    frame_object_ids: Option<Vec<u64>>,
    current_point_lights: Vec<crate::light::PointLight>,
    taa_frame_count: u64,
    taa_history_read_idx: usize,
    taa_prev_vp_unjittered: Mat4,
    vsm_request_frames: u32,
    vsm_shadow_frame_count: u64,
    /// Copies of the temporal textures, taken before the faces overwrite them.
    temporal: Vec<(TemporalTexture, wgpu::Texture)>,
}

/// Copies the whole texture `from` into `to` (same size and format).
fn copy_whole_texture(
    encoder: &mut wgpu::CommandEncoder,
    from: &wgpu::Texture,
    to: &wgpu::Texture,
    aspect: wgpu::TextureAspect,
) {
    encoder.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo {
            texture: from,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect,
        },
        wgpu::TexelCopyTextureInfo {
            texture: to,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect,
        },
        from.size(),
    );
}

impl Renderer {
    /// Saves the state a probe capture changes and switches the capture guards on.
    ///
    /// Must be paired with [`Self::end_probe_capture`] on the same renderer.
    pub(crate) fn begin_probe_capture(&mut self) -> ProbeCaptureState {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("reflection_probe_backup"),
            });
        let temporal = {
            let renderer: &Renderer = self;
            TemporalTexture::ALL
                .into_iter()
                .map(|kind| {
                    let live = kind.texture(renderer);
                    let backup = renderer.device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("reflection_probe_temporal_backup"),
                        size: live.size(),
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: live.format(),
                        usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    });
                    copy_whole_texture(&mut encoder, live, &backup, kind.aspect());
                    (kind, backup)
                })
                .collect::<Vec<_>>()
        };
        self.queue.submit(Some(encoder.finish()));
        self.probe_capture_active = true;
        ProbeCaptureState {
            camera_uniform: self.camera_uniform,
            light_uniform: self.light_uniform,
            camera_near: self.camera_near,
            camera_far: self.camera_far,
            frame_object_ids: self.frame_object_ids.clone(),
            current_point_lights: self.current_point_lights.clone(),
            taa_frame_count: self.taa_frame_count,
            taa_history_read_idx: self.taa_history_read_idx,
            taa_prev_vp_unjittered: self.taa_prev_vp_unjittered,
            vsm_request_frames: self.vsm_request_frames,
            vsm_shadow_frame_count: self.vsm_shadow_frame_count,
            temporal,
        }
    }

    /// Restores the state saved by [`Self::begin_probe_capture`] and switches the guards off.
    pub(crate) fn end_probe_capture(&mut self, state: ProbeCaptureState) {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("reflection_probe_restore"),
            });
        for (kind, backup) in &state.temporal {
            copy_whole_texture(&mut encoder, backup, kind.texture(self), kind.aspect());
        }
        self.queue.submit(Some(encoder.finish()));

        self.camera_uniform = state.camera_uniform;
        self.update_camera_buffer();
        self.update_light_buffer(&state.light_uniform);
        self.camera_near = state.camera_near;
        self.camera_far = state.camera_far;
        self.frame_object_ids = state.frame_object_ids;
        self.current_point_lights = state.current_point_lights;
        self.taa_frame_count = state.taa_frame_count;
        self.taa_history_read_idx = state.taa_history_read_idx;
        self.taa_prev_vp_unjittered = state.taa_prev_vp_unjittered;
        self.vsm_request_frames = state.vsm_request_frames;
        self.vsm_shadow_frame_count = state.vsm_shadow_frame_count;
        self.probe_capture_active = false;
    }
}
