//! GPU particle upload and debug-line drawing.

use super::*;

impl Renderer {
    /// Creates the optional GPU particle system (`capacity` living particles).
    pub fn init_particle_system(&mut self, capacity: u32) {
        let hdr_format = wgpu::TextureFormat::Rgba16Float;
        self.particle_system = Some(ParticleGpuSystem::new(&self.device, capacity, hdr_format));
        self.ensure_reactive_texture();
    }

    /// Upload CPU-simulated particles from `redox_vfx` for GPU billboard draw.
    pub fn upload_particles_from_vfx(&mut self, system: &ParticleSystem) {
        self.particle_sim_uniforms = system.sim_uniforms(1.0 / 60.0);
        if let Some(ps) = &mut self.particle_system {
            ps.upload_from_vfx(&self.device, &self.queue, system, &self.camera_uniform);
        }
    }

    /// Sets debug lines to draw this frame (e.g. from `redox_audio::AudioDebugDraw`).
    pub fn set_debug_lines(&mut self, lines: Vec<(Vec3, Vec3, bool)>) {
        self.current_debug_lines = lines;
    }

    /// Uploads current debug lines to the GPU. Call before starting the render pass that will draw them.
    pub fn upload_debug_lines(&mut self) {
        let lines = std::mem::take(&mut self.current_debug_lines);
        if !lines.is_empty() {
            self.debug_lines_pass.upload_lines(&self.queue, &lines);
        }
    }

    /// Records drawing the debug lines uploaded by [`upload_debug_lines`](Self::upload_debug_lines).
    /// Call after [`record_draw`](Self::record_draw) in the same render pass. Uses only `&self`.
    pub fn draw_debug_lines<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        self.debug_lines_pass.draw(&self.camera_bind_group, pass);
    }

    /// If debug lines were set this frame, uploads and records drawing them into `pass`.
    /// Call after [`record_draw`](Self::record_draw) in the same render pass. Clears the line list.
    pub fn draw_debug_lines_if_any<'a>(&'a mut self, pass: &mut wgpu::RenderPass<'a>) {
        let lines = std::mem::take(&mut self.current_debug_lines);
        if lines.is_empty() {
            return;
        }
        self.debug_lines_pass.upload_lines(&self.queue, &lines);
        self.debug_lines_pass.draw(&self.camera_bind_group, pass);
    }
}
