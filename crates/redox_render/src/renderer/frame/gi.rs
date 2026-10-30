//! GI passes: DDGI irradiance fill (moved verbatim from frame.rs; future GI passes go here).

use super::super::*;
use super::FrameState;

impl Renderer {
    pub(super) fn pass_ddgi_fill(&mut self, f: &mut FrameState<'_>) {
        self.gi.poll(&self.device, &self.queue);
        let objects = f.objects;
        let encoder = &mut f.encoder;
        self.ddgi_gpu.record_gpu_fill(
            &self.device,
            &self.queue,
            encoder,
            objects,
            -Vec3::from_array([
                self.light_uniform.dir_direction[0],
                self.light_uniform.dir_direction[1],
                self.light_uniform.dir_direction[2],
            ]),
            Vec3::from_array([
                self.light_uniform.dir_color[0],
                self.light_uniform.dir_color[1],
                self.light_uniform.dir_color[2],
            ]),
            self.light_uniform.dir_color[3],
        );
    }
}
