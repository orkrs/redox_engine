//! **Prism Vault** — mini-game showcasing RedOx Uplift v2.
//!
//! Collect 8 prismatic crystals while avoiding StateTree-driven sentinels.
//! World Partition streams an arena around the player; VFX + audio fire on dash/pickup.
//!
//! Run: `cargo run --example prism_vault -p redox_core`
//!
//! WASD move · Q/E camera · Shift dash · R restart · F1 debug · ESC quit

use std::f32::consts::TAU;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use kira::Frame;
use kira::sound::static_sound::{StaticSoundData, StaticSoundSettings};
use redox_audio::{AudioContext, MixerBusId, with_audio};
use redox_core::config::EngineConfig;
use redox_core::window::create_window;
use redox_ecs::{Entity, World};
use redox_gameplay::{
    Ability, ActiveEffects, AttributeSet, GameplayEffect, StateTreeComponent,
    ability_cooldown_system, gameplay_effect_system,
};
use redox_input::InputState;
use redox_math::{Quat, Vec3};
use redox_render::camera::Camera;
use redox_render::context::RenderContext;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene, PointLight};
use redox_render::material::Material;
use redox_render::mesh::primitive::{create_cube, create_sphere};
use redox_render::systems::{RenderObject, Transform};
use redox_scene::{
    PcgKind, PcgParams, StreamedActor, StreamingReport, WorldPartition, generate_to, tick_streaming,
};
use redox_ui::context::UiContext;
use redox_vfx::{ParticleEmitter, ParticleEmitterComponent, ParticleSystem};
use winit::keyboard::KeyCode;

mod hud;
mod render;
mod tick;
mod world;

use hud::*;
use render::*;
use tick::*;
use world::*;

const CRYSTAL_COUNT: usize = 8;
const ARENA: f32 = 34.0;
const GOD_MODE: bool = true;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Playing,
    Won,
    Lost,
}

#[derive(Clone, Copy)]
struct Renderable {
    mesh_index: usize,
    material_index: usize,
}

struct PlayerMarker;

struct Crystal {
    spin: f32,
}

struct Sentinel {
    hub: Vec3,
    orbit_radius: f32,
    angle: f32,
    speed: f32,
}

struct ArenaAssets {
    cube: usize,
    sphere: usize,
    sentinel_mat: usize,
    player_mat: usize,
    crystal_mat: usize,
    stream_prop_mat: usize,
    stream_wall_mat: usize,
}

struct Session {
    phase: Phase,
    player: Entity,
    crystals: Vec<Entity>,
    sentinels: Vec<(Entity, Sentinel)>,
    collected: u8,
    health: f32,
    stamina: f32,
    camera_yaw: f32,
    dash_timer: f32,
    dash_dir: Vec3,
    particles: ParticleSystem,
    dash_emitter: ParticleEmitterComponent,
    pickup_emitter: ParticleEmitterComponent,
    time_played: f32,
    best_time: f32,
    last_stream: StreamingReport,
    stream_bytes: usize,
}

fn main() {
    env_logger::init();

    let config = EngineConfig {
        window_title: "RedOx — Prism Vault".to_string(),
        window_width: 1280,
        window_height: 720,
        render_scale: 1.0,
        aa_sharp: true,
        ..Default::default()
    };

    let (event_loop, window) = create_window(&config);
    let mut ctx = pollster::block_on(RenderContext::new(
        window.clone(),
        config.wgpu_present_mode(),
    ));
    ctx.set_render_scale(config.render_scale);
    ctx.set_aa_sharp(true);
    ctx.set_rcas_sharpness(0.28);
    ctx.set_exposure(1.15);
    // Match editor: VSM-only directional shadows + GTAO (ssao_enabled drives GTAO pass).
    ctx.set_vsm_enabled(true);
    ctx.set_ssao_enabled(true);
    ctx.init_particle_system(4096);

    let mut ui = UiContext::new(&window, ctx.device(), ctx.surface_format());
    ui.show_debug = false;
    let mut world = World::new();
    world.insert_resource(InputState::new());

    {
        let audio = with_audio(&mut world);
        register_procedural_sfx(audio);
        audio.sync_mixer();
    }

    let partition = open_or_generate_partition();
    world.insert_resource(partition);

    let assets = build_arena(&mut world, &mut ctx);
    let mut session = spawn_run(&mut world, &assets);
    let mut use_vsm = true;
    let mut use_scatter = false;
    let mut render_scale = config.render_scale;
    let mut show_debug = false;
    let mut last = Instant::now();

    event_loop
        .run(move |event, elwt| match event {
            winit::event::Event::WindowEvent { event, .. } => {
                if let Some(input) = world.get_resource_mut::<InputState>() {
                    input.process_window_event(&event);
                }
                if ui.handle_window_event(&window, &event) {
                    return;
                }
                match event {
                    winit::event::WindowEvent::CloseRequested => elwt.exit(),
                    winit::event::WindowEvent::Resized(size) => ctx.resize(size.width, size.height),
                    winit::event::WindowEvent::RedrawRequested => {
                        let dt = last.elapsed().as_secs_f32().min(0.05);
                        last = Instant::now();

                        if world
                            .get_resource::<InputState>()
                            .is_some_and(|i| i.keyboard.is_pressed(KeyCode::Escape))
                        {
                            elwt.exit();
                        }
                        if world
                            .get_resource::<InputState>()
                            .is_some_and(|i| i.keyboard.just_pressed(KeyCode::F1))
                        {
                            show_debug = !show_debug;
                            ui.show_debug = show_debug;
                        }

                        let input = world.get_resource::<InputState>().cloned();
                        tick_game(&mut world, &assets, &mut session, input.as_ref(), dt);
                        ctx.upload_particles_from_vfx(&session.particles);
                        sync_hud_stats(&mut session, &world);
                        ability_cooldown_system(&mut world, dt);
                        gameplay_effect_system(&mut world, dt);
                        sync_hud_stats(&mut session, &world);

                        if (ctx.render_scale - render_scale).abs() > 0.001 {
                            ctx.set_render_scale(render_scale);
                        }
                        ctx.set_vsm_enabled(use_vsm);
                        ctx.set_lighting_mode(if use_scatter {
                            redox_render::LightingMode::ScatterLight
                        } else {
                            redox_render::LightingMode::Clustered
                        });

                        let (cam_pos, cam_rot, camera) = update_camera(&world, &session, &ctx);
                        ctx.camera_uniform.update(&camera, cam_pos, cam_rot);
                        ctx.update_camera_buffer();

                        let lights = build_lights(session.time_played, use_vsm);
                        let objects = extract_objects(&world, &assets, &ctx);
                        let fv = FrameView::new(&camera, &objects, &lights);

                        let output = ctx.surface().get_current_texture().unwrap();
                        let view = output.texture.create_view(&Default::default());
                        ctx.render_frame_view(&fv, &view).unwrap();

                        ui.record_frame(dt);
                        ui.begin_frame(&window);
                        draw_hud(
                            &ui.egui_ctx,
                            &session,
                            &world,
                            dt,
                            show_debug,
                            &mut use_vsm,
                            &mut use_scatter,
                            &mut render_scale,
                            &ctx,
                        );
                        ui.draw_debug(&mut world);
                        let mut enc = ctx.device.create_command_encoder(&Default::default());
                        ui.end_frame_and_render(
                            ctx.device(),
                            ctx.queue(),
                            &mut enc,
                            &window,
                            &view,
                            egui_wgpu::ScreenDescriptor {
                                size_in_pixels: [ctx.config.width, ctx.config.height],
                                pixels_per_point: window.scale_factor() as f32,
                            },
                        );
                        ctx.queue.submit(Some(enc.finish()));
                        output.present();

                        // After gameplay reads `just_pressed`. Doing this in
                        // `NewEvents` dropped R: the next cycle's begin_frame
                        // ran before RedrawRequested.
                        if let Some(input) = world.get_resource_mut::<InputState>() {
                            input.begin_frame();
                        }
                    }
                    _ => {}
                }
            }
            winit::event::Event::AboutToWait => window.request_redraw(),
            _ => {}
        })
        .unwrap();
}
