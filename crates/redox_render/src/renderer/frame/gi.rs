//! GI passes: the probe update that feeds the irradiance and visibility atlases read by PBR.

use super::super::*;
use super::FrameState;
use crate::gi::card_light::{CardPointGpu, MAX_CARD_POINTS};
use crate::gi::probes::{GiDebug, GiFrame};

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
        let points: Vec<CardPointGpu> = self.light_points_for_cards();
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
                cards: self.gi.cards.as_ref(),
                points: &points,
            },
        );
        if self.gi_probes.debug() == GiDebug::Probes {
            self.current_debug_lines = self.gi_probes.probe_lines();
        }
    }

    /// Point lights of the light uniform in the layout the card light pass reads.
    fn light_points_for_cards(&self) -> Vec<CardPointGpu> {
        let l = &self.light_uniform;
        let count = (l.num_point_lights as usize).min(MAX_CARD_POINTS);
        (0..count)
            .map(|i| CardPointGpu {
                pos_intensity: l.point_lights_pos[i],
                color_radius: l.point_lights_color[i],
            })
            .collect()
    }
}
