//! Audio Debug panel for the debug overlay.
//!
//! Shows reverb zones, spatial emitters, occlusion coefficients, and toggles
//! for 3D visualization of occlusion rays and reverb zone bounds.

use redox_audio::{AudioDebugDraw, AudioListener, ReverbZone, SpatialAudioEmitter};
use redox_ecs::world::World;
use redox_math::Transform;

/// Audio debug panel state.
pub struct AudioDebugPanel {
    /// Whether the window is open.
    pub open: bool,
}

impl AudioDebugPanel {
    pub fn new() -> Self {
        Self { open: true }
    }

    /// Draws the Audio Debug window. Call once per egui frame.
    pub fn show(&mut self, ctx: &egui::Context, world: &mut World) {
        let mut open = self.open;
        egui::Window::new("🔊 Audio Debug")
            .open(&mut open)
            .resizable(true)
            .default_pos([400.0, 320.0])
            .default_size([320.0, 380.0])
            .show(ctx, |ui| {
                self.draw_contents(ui, world);
            });
        self.open = open;
    }

    fn draw_contents(&mut self, ui: &mut egui::Ui, world: &mut World) {
        let mut draw_rays = false;
        let mut draw_zones = false;
        if let Some(debug) = world.get_resource::<AudioDebugDraw>() {
            draw_rays = debug.draw_rays;
            draw_zones = debug.draw_zones;
        }

        ui.heading("Visualization");
        ui.checkbox(&mut draw_rays, "Draw occlusion rays (listener → emitter)");
        ui.checkbox(&mut draw_zones, "Draw reverb zone bounds");
        ui.weak("Rays: green = clear, red = occluded");
        ui.separator();

        if let Some(debug) = world.get_resource_mut::<AudioDebugDraw>() {
            debug.draw_rays = draw_rays;
            debug.draw_zones = draw_zones;
        }

        ui.heading("Reverb zones");
        let mut zone_count = 0u32;
        for entity in world.all_entities() {
            if let Some(zone) = world.get_component::<ReverbZone>(entity) {
                zone_count += 1;
                let pos = world
                    .get_component::<Transform>(entity)
                    .map(|t| format!("({:.1}, {:.1}, {:.1})", t.translation.x, t.translation.y, t.translation.z))
                    .unwrap_or_else(|| "—".to_string());
                ui.horizontal(|ui| {
                    ui.label(format!("• {} @ {}", zone.preset_name, pos));
                    if zone.listener_inside {
                        ui.label(egui::RichText::new("inside").color(egui::Color32::GREEN));
                    }
                });
            }
        }
        if zone_count == 0 {
            ui.weak("No reverb zones in scene");
        }
        ui.separator();

        ui.heading("Spatial emitters (occlusion)");
        let mut emitter_count = 0u32;
        for entity in world.all_entities() {
            if let Some(emitter) = world.get_component::<SpatialAudioEmitter>(entity) {
                emitter_count += 1;
                let occl = emitter.occlusion_coefficient;
                let occluded = occl > 0.001;
                let color = if occluded {
                    egui::Color32::from_rgb(220, 80, 80)
                } else {
                    egui::Color32::from_rgb(80, 200, 120)
                };
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "Entity {:?}  occlusion: {:.2}  {}",
                        entity.id(),
                        occl,
                        if occluded { "🔴 blocked" } else { "🟢 clear" }
                    ));
                    ui.label(egui::RichText::new("").color(color));
                });
            }
        }
        if emitter_count == 0 {
            ui.weak("No spatial emitters in scene");
        }
        ui.separator();

        if let Some(listener) = world
            .all_entities()
            .find(|&e| world.get_component::<AudioListener>(e).is_some())
            .and_then(|e| world.get_component::<AudioListener>(e))
        {
            ui.weak(format!(
                "Listener @ ({:.1}, {:.1}, {:.1})",
                listener.position.x, listener.position.y, listener.position.z
            ));
        }
    }
}

impl Default for AudioDebugPanel {
    fn default() -> Self {
        Self::new()
    }
}
