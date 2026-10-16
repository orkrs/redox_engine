//! Anti-aliasing controls (TAA, sharpening, reactive mask).

use super::*;

impl Renderer {
    /// Enables or disables Temporal Anti-Aliasing.
    ///
    /// When enabled the pipeline inserts a velocity pass and a TAA accumulation
    /// pass between the main forward pass and tone mapping.  The jitter is
    /// injected automatically inside [`render_frame`]; callers keep their
    /// `update_camera_buffer` workflow unchanged.
    ///
    /// Disabling TAA resets the frame counter so history is correctly
    /// re-initialised if TAA is re-enabled later.
    pub fn enable_taa(&mut self, enabled: bool) {
        if enabled {
            self.aa_sharp = false;
            self.taa_frame_count = 0;
            self.taa_history_read_idx = 0;
            self.taa_prev_vp_unjittered = Mat4::from_cols_array_2d(&self.camera_uniform.view_proj);
        } else if self.taa_enabled {
            self.taa_frame_count = 0;
            self.taa_history_read_idx = 0;
        }
        self.taa_enabled = enabled;
        self.sync_msaa_sample_count();
    }

    /// Enables the Sharp AA pipeline (MSAA + Smart Resolve + SMAA + RCAS).
    /// Mutually exclusive with TAA.
    pub fn set_aa_sharp(&mut self, enabled: bool) {
        self.aa_sharp = enabled;
        if enabled {
            self.taa_enabled = false;
            self.taa_frame_count = 0;
            self.taa_history_read_idx = 0;
        }
        self.sync_msaa_sample_count();
    }

    /// Sets TAA blend factor (weight of current frame).
    ///
    /// - Lower values reduce aliasing/shimmer, but increase ghosting risk.
    /// - Typical range: 0.04–0.10.
    pub fn set_taa_blend_alpha(&mut self, alpha: f32) {
        self.taa_blend_alpha = alpha.clamp(0.02, 0.20);
        // Clear accumulation once so the new blend behaviour settles quickly.
        self.taa_frame_count = 0;
        self.taa_history_read_idx = 0;
    }

    /// Sets RCAS sharpening strength after TAA (0 = max sharp, ~0.5 = mild).
    pub fn set_rcas_sharpness(&mut self, sharpness_stops: f32) {
        self.rcas_sharpness = sharpness_stops.clamp(0.0, 1.0);
    }

    /// Reactive mask view for TAA (real R8 texture or 1×1 fallback).
    pub fn reactive_mask_view(&self) -> &wgpu::TextureView {
        self.reactive_view
            .as_ref()
            .unwrap_or(&self.taa_pass.reactive_fallback_view)
    }

    /// Allocates the internal-resolution R8 reactive mask used by particles / TAA.
    pub fn ensure_reactive_texture(&mut self) {
        let size = wgpu::Extent3d {
            width: self.internal_width.max(1),
            height: self.internal_height.max(1),
            depth_or_array_layers: 1,
        };
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("reactive_mask"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        self.reactive_texture = Some(tex);
        self.reactive_view = Some(view);
        self.rebuild_taa_history_bind_groups();
    }

    pub(super) fn rebuild_taa_history_bind_groups(&mut self) {
        let reactive = self.reactive_mask_view();
        self.taa_history_bgs = [
            self.taa_pass.create_bind_group(
                &self.device,
                &self.hdr_view,
                &self.taa_velocity_view,
                &self.taa_history_view[0],
                reactive,
                &self.prev_depth_view,
            ),
            self.taa_pass.create_bind_group(
                &self.device,
                &self.hdr_view,
                &self.taa_velocity_view,
                &self.taa_history_view[1],
                reactive,
                &self.prev_depth_view,
            ),
        ];
    }

    pub(super) fn rebuild_sharp_bind_groups(&mut self) {
        self.tone_mapping_ldr_bg = create_tonemap_bind_group(
            &self.device,
            &self.tone_mapping_ldr_pipeline,
            &self.hdr_view,
            &self.bloom_mip_views[0],
            &self.hdr_sampler,
            &self.tonemap_uniform_buffer,
        );
        self.tone_mapping_rcas_bg = create_tonemap_bind_group(
            &self.device,
            &self.rcas_present_pipeline,
            &self.smaa_out_view,
            &self.bloom_mip_views[0],
            &self.hdr_sampler,
            &self.tonemap_rcas_uniform_buffer,
        );
    }
}
