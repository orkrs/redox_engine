//! egui HUD overlay.

use super::*;

pub(crate) fn draw_hud(
    egui: &egui::Context,
    session: &Session,
    world: &World,
    dt: f32,
    show_debug: bool,
    use_vsm: &mut bool,
    use_scatter: &mut bool,
    render_scale: &mut f32,
    ctx: &RenderContext,
) {
    egui::Area::new(egui::Id::new("hud"))
        .anchor(egui::Align2::LEFT_TOP, [14.0, 14.0])
        .show(egui, |ui| {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.heading("Prism Vault");
                ui.label(format!("Crystals: {}/{}", session.collected, CRYSTAL_COUNT));
                ui.label(format!("Time: {:.1}s", session.time_played));
                if session.best_time < 1.0e6 {
                    ui.label(format!("Best: {:.1}s", session.best_time));
                }
                ui.add(egui::ProgressBar::new(session.health / 100.0).text("HP"));
                ui.add(egui::ProgressBar::new(session.stamina / 100.0).text("Stamina"));
                if session.phase == Phase::Won {
                    ui.colored_label(egui::Color32::from_rgb(80, 220, 140), "VAULT CLEARED!");
                } else if session.phase == Phase::Lost {
                    ui.colored_label(egui::Color32::from_rgb(240, 90, 90), "BREACH — press R");
                }
                ui.separator();
                ui.label("WASD · Q/E · Shift dash · R · F1 debug");
            });
        });

    if show_debug {
        egui::Window::new("Engine stack")
            .default_pos([14.0, 220.0])
            .default_width(360.0)
            .show(egui, |ui| {
                ui.monospace(format!("fps: {:.0}", 1.0 / dt.max(1e-4)));
                ui.monospace(format!(
                    "internal: {}×{}  TSR: {} (TAA upscale when scale<1)",
                    ctx.internal_width,
                    ctx.internal_height,
                    ctx.uses_tsr()
                ));
                ui.checkbox(use_vsm, "VSM directional (clipmap + temporal SMRT)");
                ui.checkbox(use_scatter, "ScatterLight (K=8 stochastic points)");
                ui.add(
                    egui::Slider::new(render_scale, 0.5..=1.0)
                        .text("render scale")
                        .step_by(0.05),
                );
                ui.horizontal(|ui| {
                    if ui.button("TSR 0.85 (Quality)").clicked() {
                        *render_scale = 0.85;
                    }
                    if ui.button("Native 1.0").clicked() {
                        *render_scale = 1.0;
                    }
                });
                ui.separator();
                ui.monospace(format!(
                    "TSR: {}",
                    if ctx.uses_tsr() {
                        "active (sub-native + TAA)"
                    } else {
                        "idle (native or TAA off)"
                    }
                ));
                ui.monospace(format!(
                    "GTAO: {}  VSM: {}  dirty_pages={}",
                    if ctx.ssao_enabled { "ON" } else { "OFF" },
                    if ctx.vsm_enabled { "ON" } else { "OFF" },
                    ctx.vsm_dirty_pages_last_frame
                ));
                ui.monospace(format!(
                    "ScatterLight: {}",
                    if ctx.uses_scatter_light() {
                        "ON"
                    } else {
                        "OFF (clustered)"
                    }
                ));
                let ddgi = &ctx.ddgi_gpu.grid;
                ui.monospace(format!(
                    "DDGI: {}×{}×{} trilinear+oct (CPU probe fill → GPU sample; no RT)",
                    ddgi.dimensions.x, ddgi.dimensions.y, ddgi.dimensions.z
                ));
                ui.monospace(format!(
                    "SDF: atlas {}³ enabled={} (8-slot page table; demo bricks)",
                    redox_render::sdf::GPU_ATLAS_EXTENT,
                    ctx.sdf_gpu.enabled
                ));
                ui.monospace("GTAO: slice AO + bent normals → 5×5 bilateral blur → upsample");
                ui.monospace("Dir. shadow: VSM trace (1 Vogel ray) → temporal filter → PBR sample");
                ui.monospace(
                    "Bloom: 5-mip downsample → tent upsample → composite (knee 1.2, *0.25)",
                );
                ui.monospace("Substrate: eval_substrate_lit in pbr.wgsl (VG resolve = base lobe)");

                if let Some(vg) = &ctx.vg_system {
                    let s = vg.stats();
                    ui.monospace(format!(
                        "VG: {}/{} meshlets  tris {}/{}",
                        s.visible_meshlets,
                        s.total_meshlets,
                        s.visible_triangles,
                        s.total_triangles
                    ));
                } else {
                    ui.monospace("VG: runtime not initialised");
                }

                ui.separator();
                if let Some(part) = world.get_resource::<WorldPartition>() {
                    let cell = part.cell_at(
                        world
                            .get_component::<Transform>(session.player)
                            .map(|t| t.translation)
                            .unwrap_or(Vec3::ZERO),
                    );
                    ui.monospace(format!(
                        "WorldPartition: cell ({},{})  loaded={}  entities={}",
                        cell.x,
                        cell.y,
                        part.loaded_cells().count(),
                        part.loaded_entity_count()
                    ));
                    ui.monospace(format!(
                        "  last tick ±L/U: {}/{}  reloaded={}",
                        session.last_stream.loaded.len(),
                        session.last_stream.unloaded.len(),
                        session.last_stream.reloaded.len()
                    ));
                } else {
                    ui.monospace("WorldPartition: missing resource");
                }

                let mut chase = 0usize;
                let mut patrol = 0usize;
                for (e, _) in &session.sentinels {
                    match world
                        .get_component::<StateTreeComponent>(*e)
                        .map(|a| a.tree.active_name())
                    {
                        Some("Chase") => chase += 1,
                        Some("Patrol") => patrol += 1,
                        _ => {}
                    }
                }
                ui.monospace(format!(
                    "StateTree sentinels: Patrol={patrol} Chase={chase}"
                ));

                ui.separator();
                if let Some(audio) = world.get_resource::<AudioContext>() {
                    ui.monospace(format!(
                        "Audio: device={}  SFX gain={:.2}",
                        audio.is_available(),
                        audio.mixer.routed_gain(MixerBusId::Sfx)
                    ));
                }
                ui.monospace(format!(
                    "VFX ParticleSystem: alive={}  upload_bytes={}",
                    session.particles.alive_count(),
                    session.stream_bytes
                ));
            });
    }
}
