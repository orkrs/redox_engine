//! Runtime settings: environment, VSM, SSAO, exposure, skybox, global bind group.

use super::*;

impl Renderer {
    /// Sets the environmental skybox and generates IBL maps.
    pub fn set_environment(&mut self, equirect: &Texture) {
        self.ibl_resource =
            IBLResource::from_equirect(&self.device, &self.queue, &self.ibl_processor, equirect);
        self.update_shader_debug_buffer(ShaderDebugVizMode::None);
        self.update_global_bind_group();
        // The skybox samples the environment view directly: rebind it so a
        // live skybox picks up the freshly generated cubemap.
        if let Some(sb) = &mut self.skybox_pass {
            sb.refresh_bind_group(
                &self.device,
                &self.camera_buffer,
                &self.ibl_resource.environment_view,
                &self.ibl_processor.sampler,
            );
        }
    }

    /// Sets the linear exposure multiplier used by the ACES tone mapper.
    ///
    /// `1.0` is neutral; larger values brighten the frame. Takes effect on
    /// the next rendered frame (the uniform is written per frame).
    pub fn set_vsm_enabled(&mut self, enabled: bool) {
        if enabled && !self.vsm_enabled {
            // Drop stale page cache so the next frame rebuilds with current matrices.
            self.vsm_clipmap
                .invalidate(&[crate::shadow::cache::InvalidationEvent::Full]);
            self.vsm_warm = false;
            self.reset_vsm_shadow_temporal();
        }
        self.vsm_enabled = enabled;
    }
    /// Marks directional (VSM) shadow pages under `bounds` stale so they re-render next frame.
    ///
    /// The VSM caches pages until an object moves more than 0.5 m; call this every frame for
    /// geometry that deforms in place (skinning) so its sun shadow follows the animation.
    pub fn invalidate_shadow_region(&mut self, bounds: &[Aabb]) {
        if self.vsm_enabled {
            self.vsm_clipmap.invalidate_aabbs(bounds);
        }
    }

    /// Clears VSM screen-space shadow temporal history (first frame = no blend).
    pub(super) fn reset_vsm_shadow_temporal(&mut self) {
        self.vsm_shadow_frame_count = 0;
    }

    /// Enables screen-space VSM trace (1 ray + temporal) vs inline multi-ray PBR.
    pub fn set_vsm_screen_shadows(&mut self, enabled: bool) {
        if enabled != self.vsm_screen_shadows {
            self.vsm_screen_shadows = enabled;
            self.reset_vsm_shadow_temporal();
            // Keep LightUniform flag in sync next frame (written in render_frame)
            self.light_uniform.vsm_screen_shadows = u32::from(enabled);
        }
    }


    /// Sets SMRT quality for VSM soft shadows (rays × samples per pixel).
    pub fn set_smrt_quality(&mut self, quality: crate::shadow::smrt::SmrtQuality) {
        self.smrt_config = quality.config();
    }

    /// Sets temporal blend alpha for VSM screen shadows (weight of current frame).
    /// Lower = smoother but more ghosting; typical 0.05..0.20. Default 0.08 (92% history).
    pub fn set_vsm_temporal_alpha(&mut self, alpha: f32) {
        self.vsm_shadow_temporal_alpha = alpha.clamp(0.05, 0.20);
    }


    pub fn set_ssao_enabled(&mut self, enabled: bool) {
        self.ssao_enabled = enabled;
    }

    pub fn set_exposure(&mut self, exposure: f32) {
        self.exposure = exposure.max(0.0);
    }

    /// Enables or disables the skybox background pass.
    ///
    /// When enabled, the pass samples `ibl_resource.environment_view` in the
    /// forward pass before opaque geometry. While only the dummy IBL resource
    /// exists (no [`Self::set_environment`] call yet) the pass is kept but
    /// skipped at draw time — the placeholder cubemap is all black.
    pub fn set_skybox(&mut self, enabled: bool) {
        if enabled {
            if self.skybox_pass.is_none() {
                let sb = SkyboxPass::new(
                    &self.device,
                    &self.camera_buffer,
                    &self.ibl_resource.environment_view,
                    &self.ibl_processor.sampler,
                    wgpu::TextureFormat::Rgba16Float,
                    self.msaa_sample_count,
                );
                self.skybox_pass = Some(sb);
            }
        } else {
            self.skybox_pass = None;
        }
    }

    /// Recreates the PBR frame and scene bind groups with current resources.
    pub fn update_global_bind_group(&mut self) {
        self.frame_bind_group = self.pbr_pass.create_frame_bind_group(
            &self.device,
            &self.camera_buffer,
            &self.shader_debug_buffer,
            &self.ibl_resource.irradiance_view,
            &self.ibl_resource.prefiltered_view,
            &self.ibl_resource.brdf_lut_view,
            &self.ibl_processor.sampler,
            &self.blue_noise_view,
            &self.blue_noise_sampler,
        );
        self.scene_bind_group = self.pbr_pass.create_scene_bind_group(
            &self.device,
            &self.light_buffer,
            &self.shadow_view,
            &self.shadow_sampler,
            &self.ssao_blurred_view,
            &self.ssao_pbr_sampler,
            &self.cluster_manager.point_lights_buffer,
            &self.cluster_manager.metadata_buffer,
            &self.cluster_manager.light_indices_buffer,
            &self.cluster_manager.info_buffer,
            &self.csm_view,
            &self.csm_sampler,
            &self.local_shadow_atlas_view,
            &self.shadow_uniform_buffer,
            &self.vsm_pages_array_view,
            &self.vsm_sampler,
            &self.vsm_info_buffer,
            &self.ddgi_gpu.grid_buffer,
            &self.ddgi_gpu.probe_buffer,
            &self.sdf_gpu.scene_buffer,
            &self.sdf_gpu.view,
            &self.sdf_gpu.sampler,
            &self.vsm_shadow_history_view,
            &self.vsm_shadow_pbr_sampler,
        );
    }
}
