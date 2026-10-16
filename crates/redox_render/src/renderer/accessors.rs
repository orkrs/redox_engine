//! Simple accessors and render-quality setters.

use super::*;

impl Renderer {
    /// Enable or disable multi-threaded command recording (CSM + prepass).
    pub fn set_mt_command_recording(&mut self, enabled: bool) {
        self.mt_command_recording = enabled;
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    pub fn surface(&self) -> &wgpu::Surface<'_> {
        self.surface
            .as_ref()
            .expect("offscreen Renderer has no window surface")
    }

    pub fn config(&self) -> &wgpu::SurfaceConfiguration {
        &self.config
    }

    /// Sets the internal render scale used by TAAU.
    ///
    /// - < 1.0: TAAU upscaling (faster, blurrier, more aliasing risk)
    /// - = 1.0: regular TAA (no upscale)
    /// - > 1.0: temporal supersampling (sharper, fewer stair-steps, slower)
    ///
    /// This recreates internal-resolution textures (depth/HDR/normal/SSAO/velocity)
    /// and forces a TAA history reset on the next frame.
    pub fn set_render_scale(&mut self, scale: f32) {
        self.render_scale = scale.clamp(0.5, 2.0);
        // Recreate size-dependent resources using the current output resolution.
        let w = self.config.width;
        let h = self.config.height;
        self.resize(w, h);
        // If TAA is enabled, ensure history is clean after scale change.
        self.taa_frame_count = 0;
        self.taa_history_read_idx = 0;
        self.reset_vsm_shadow_temporal();
    }

    /// Selects the forward lighting path (clustered vs ScatterLight).
    pub fn set_lighting_mode(&mut self, mode: LightingMode) {
        self.lighting_mode = mode;
    }

    /// Returns `true` when ScatterLight stochastic sampling is active.
    pub fn uses_scatter_light(&self) -> bool {
        self.lighting_mode == LightingMode::ScatterLight
    }

    /// Returns `true` when TAA upscales from sub-native resolution (TSR path).
    pub fn uses_tsr(&self) -> bool {
        self.taa_enabled && self.render_scale < 0.999
    }
}
