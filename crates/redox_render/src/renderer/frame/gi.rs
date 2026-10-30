//! GI passes: the probe update that feeds the irradiance and visibility atlases read by PBR.

use super::super::*;
use super::FrameState;
use crate::gi::probes::GiFrame;

impl Renderer {
    /// Traces the probe cascades and updates their atlases. Runs before the lighting passes
    /// that read the stable atlas textures.
    pub(super) fn pass_gi_update(&mut self, f: &mut FrameState<'_>) {
        f.touch("gi_irradiance");
        self.gi.poll(&self.device, &self.queue);
        let l = &self.light_uniform;
        let sun_radiance =
            Vec3::new(l.dir_color[0], l.dir_color[1], l.dir_color[2]) * l.dir_color[3];
        let sun_dir = -Vec3::new(l.dir_direction[0], l.dir_direction[1], l.dir_direction[2]);
        let camera = self.camera_uniform.camera_pos;
        self.gi_probes.update(
            &self.device,
            &self.queue,
            &mut f.encoder,
            &GiFrame {
                camera_pos: Vec3::new(camera[0], camera[1], camera[2]),
                sun_dir: sun_dir.normalize_or_zero(),
                sun_radiance,
                sky: &self.ibl_resource.prefiltered_view,
                sdf: &self.gi.global_sdf,
            },
        );
    }
}
