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

use kira::dsp::Frame;
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

fn arena_world_path() -> PathBuf {
    let from_manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../worlds/arena");
    if from_manifest.join("world.ron").exists() {
        return from_manifest;
    }
    PathBuf::from("worlds/arena")
}

fn open_or_generate_partition() -> WorldPartition {
    let path = arena_world_path();
    let params = PcgParams {
        name: "arena".into(),
        kind: PcgKind::Arena,
        width: 6,
        height: 6,
        cell_size: 16.0,
        seed: 42,
    };
    if !path.join("world.ron").exists() {
        if let Err(e) = generate_to(&path, &params) {
            log::warn!("PCG write failed ({e}); falling back to in-memory partition");
            let generated = redox_scene::generate(&params);
            let mut part = WorldPartition::in_memory(
                params.cell_size,
                [0.0; 3],
                generated.cells.into_iter().collect(),
            );
            configure_streaming(&mut part);
            return part;
        }
    }
    let mut part = WorldPartition::open(&path).unwrap_or_else(|e| {
        log::warn!("WorldPartition::open failed ({e}); using in-memory");
        let generated = redox_scene::generate(&params);
        WorldPartition::in_memory(
            params.cell_size,
            [0.0; 3],
            generated.cells.into_iter().collect(),
        )
    });
    configure_streaming(&mut part);
    part
}

fn configure_streaming(part: &mut WorldPartition) {
    part.config.load_radius = 1;
    part.config.unload_radius = 2;
    part.config.budget.max_loads = 2;
    part.config.budget.max_unloads = 4;
}

fn register_procedural_sfx(audio: &mut AudioContext) {
    audio.register_sfx_data("dash", tone_beep(420.0, 0.09, 0.35));
    audio.register_sfx_data("pickup", tone_beep(880.0, 0.12, 0.28));
}

fn tone_beep(freq_hz: f32, duration_secs: f32, volume: f32) -> StaticSoundData {
    let sample_rate = 22_050u32;
    let n = (duration_secs * sample_rate as f32).max(1.0) as usize;
    let frames: Vec<Frame> = (0..n)
        .map(|i| {
            let t = i as f32 / sample_rate as f32;
            let env = (1.0 - t / duration_secs).clamp(0.0, 1.0);
            let s = (t * freq_hz * TAU).sin() * volume * env;
            Frame::from_mono(s)
        })
        .collect();
    StaticSoundData {
        sample_rate,
        frames: Arc::from(frames),
        settings: StaticSoundSettings::default(),
    }
}

fn sync_hud_stats(session: &mut Session, world: &World) {
    if let Some(a) = world.get_component::<AttributeSet>(session.player) {
        session.health = a.health.current;
        session.stamina = a.stamina.current;
    }
}

fn build_arena(world: &mut World, ctx: &mut RenderContext) -> ArenaAssets {
    let cube = ctx.upload_mesh(&create_cube());
    let sphere = ctx.upload_mesh(&create_sphere(0.5, 64));

    let floor_mat = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.18, 0.17, 0.20));
        m.roughness = 1.0;
        m
    });
    let wall_mat = ctx.add_material(Material::solid(Vec3::new(0.12, 0.13, 0.18)));
    let pillar_mat = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.35, 0.32, 0.38));
        m.metallic = 0.4;
        m.roughness = 0.35;
        m
    });
    let platform_mat = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.28, 0.26, 0.32));
        m.metallic = 0.6;
        m.roughness = 0.25;
        m
    });
    let sentinel_mat = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.9, 0.15, 0.12));
        m.emissive = Vec3::new(0.6, 0.05, 0.05);
        m.metallic = 0.7;
        m.roughness = 0.2;
        m
    });
    let player_mat = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.15, 0.55, 0.95));
        m.emissive = Vec3::new(0.05, 0.15, 0.35);
        m.metallic = 0.85;
        m.roughness = 0.18;
        m
    });
    let crystal_mat = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.25, 0.75, 1.0));
        m.emissive = Vec3::new(0.5, 1.4, 2.0);
        m.metallic = 0.95;
        m.roughness = 0.08;
        m
    });
    let stream_prop_mat = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.45, 0.4, 0.32));
        m.roughness = 0.7;
        m
    });
    let stream_wall_mat = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.18, 0.2, 0.26));
        m.metallic = 0.15;
        m.roughness = 0.8;
        m
    });

    spawn_mesh(
        world,
        cube,
        floor_mat,
        Vec3::ZERO,
        Vec3::new(ARENA, 0.35, ARENA),
    );
    let half = ARENA * 0.5;
    // Low curbs, not 5 m slabs: a 34×5 wall fills the FOV whenever the orbit
    // camera looks at it and bloom turns that face into a "glowing roof".
    for &(x, z) in &[(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
        spawn_mesh(
            world,
            cube,
            wall_mat,
            Vec3::new(x * half, 0.45, z * half),
            Vec3::new(
                if x == 0.0 { ARENA } else { 0.45 },
                0.9,
                if z == 0.0 { ARENA } else { 0.45 },
            ),
        );
    }
    for (i, (px, pz)) in [(8.0, 8.0), (-9.0, 7.0), (10.0, -8.0), (-8.0, -10.0)]
        .into_iter()
        .enumerate()
    {
        spawn_mesh(
            world,
            cube,
            pillar_mat,
            Vec3::new(px, 2.2, pz),
            Vec3::new(1.4, 4.4, 1.4),
        );
        if i % 2 == 0 {
            spawn_mesh(
                world,
                cube,
                platform_mat,
                Vec3::new(px, 4.8, pz),
                Vec3::new(3.5, 0.35, 3.5),
            );
        }
    }

    ArenaAssets {
        cube,
        sphere,
        sentinel_mat,
        player_mat,
        crystal_mat,
        stream_prop_mat,
        stream_wall_mat,
    }
}

fn spawn_run(world: &mut World, assets: &ArenaAssets) -> Session {
    let player = world.spawn();
    world.add_component(
        player,
        Transform {
            translation: Vec3::new(0.0, 0.85, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::new(0.9, 1.6, 0.9),
        },
    );
    world.add_component(
        player,
        Renderable {
            mesh_index: assets.cube,
            material_index: assets.player_mat,
        },
    );
    world.add_component(player, PlayerMarker);
    world.add_component(player, AttributeSet::default_character());
    world.add_component(player, Ability::new(1, 0.9, 25.0));
    world.add_component(player, ActiveEffects::default());

    let spots: [Vec3; CRYSTAL_COUNT] = [
        Vec3::new(6.0, 1.2, 5.0),
        Vec3::new(-7.0, 1.2, 4.0),
        Vec3::new(5.0, 5.2, 8.0),
        Vec3::new(-9.0, 5.2, 7.0),
        Vec3::new(0.0, 1.2, -10.0),
        Vec3::new(11.0, 1.2, -6.0),
        Vec3::new(-11.0, 1.2, -8.0),
        Vec3::new(0.0, 1.2, 12.0),
    ];

    let mut crystals = Vec::new();
    for (i, pos) in spots.into_iter().enumerate() {
        let e = world.spawn();
        world.add_component(
            e,
            Transform {
                translation: pos,
                rotation: Quat::from_rotation_y(i as f32 * 0.7),
                scale: Vec3::splat(1.0),
            },
        );
        // Regular mesh — VG meshlet cull (cone/Hi-Z) shreds small spheres into
        // glowing shards. Keep VG for large assets, not collectible orbs.
        world.add_component(
            e,
            Renderable {
                mesh_index: assets.sphere,
                material_index: assets.crystal_mat,
            },
        );
        world.add_component(
            e,
            Crystal {
                spin: i as f32 * 0.4,
            },
        );
        crystals.push(e);
    }

    // Hubs stay off spawn: an orbit through the origin insta-kills a standing player.
    let specs = [
        (Vec3::new(12.0, 0.9, 10.0), 5.5_f32, 0.0_f32, 0.55_f32),
        (Vec3::new(-11.0, 0.9, 9.0), 5.0, TAU * 0.25, 0.75),
        (Vec3::new(9.0, 0.9, -12.0), 5.0, TAU * 0.5, 0.65),
    ];
    let mut sentinels = Vec::new();
    for (hub, radius, angle, speed) in specs {
        let e = world.spawn();
        world.add_component(
            e,
            Transform {
                translation: hub + Vec3::new(radius, 0.0, 0.0),
                rotation: Quat::IDENTITY,
                scale: Vec3::new(1.1, 1.8, 1.1),
            },
        );
        world.add_component(
            e,
            Renderable {
                mesh_index: assets.cube,
                material_index: assets.sentinel_mat,
            },
        );
        world.add_component(e, StateTreeComponent::patrol_chase());
        sentinels.push((
            e,
            Sentinel {
                hub,
                orbit_radius: radius,
                angle,
                speed,
            },
        ));
    }

    let mut dash_emitter = ParticleEmitterComponent {
        emitter: ParticleEmitter {
            spawn_rate: 120.0,
            lifetime: 0.35,
            initial_speed: 4.0,
            color: Vec3::new(0.3, 0.7, 1.0),
            size: 0.08,
            ..Default::default()
        },
        world_offset: Vec3::ZERO,
    };
    dash_emitter.emitter.max_particles = 512;
    let mut pickup_emitter = ParticleEmitterComponent {
        emitter: ParticleEmitter {
            spawn_rate: 200.0,
            lifetime: 0.6,
            initial_speed: 6.0,
            color: Vec3::new(0.4, 1.0, 1.0),
            size: 0.1,
            ..Default::default()
        },
        world_offset: Vec3::ZERO,
    };
    pickup_emitter.emitter.max_particles = 512;

    Session {
        phase: Phase::Playing,
        player,
        crystals,
        sentinels,
        collected: 0,
        health: 100.0,
        stamina: 100.0,
        camera_yaw: 0.4,
        dash_timer: 0.0,
        dash_dir: Vec3::NEG_Z,
        particles: ParticleSystem::with_capacity(4096),
        dash_emitter,
        pickup_emitter,
        time_played: 0.0,
        best_time: f32::MAX,
        last_stream: StreamingReport::default(),
        stream_bytes: 0,
    }
}

fn reset_run(world: &mut World, assets: &ArenaAssets, session: &mut Session) {
    let best = session.best_time;
    world.despawn(session.player);
    for e in session.crystals.drain(..) {
        world.despawn(e);
    }
    for (e, _) in session.sentinels.drain(..) {
        world.despawn(e);
    }
    *session = spawn_run(world, assets);
    session.best_time = best;
    session.particles.buffer_mut().clear();
}

fn tick_game(
    world: &mut World,
    assets: &ArenaAssets,
    session: &mut Session,
    input: Option<&InputState>,
    dt: f32,
) {
    if input.is_some_and(|i| i.keyboard.just_pressed(KeyCode::KeyR)) {
        reset_run(world, assets, session);
        return;
    }

    if let Some(input) = input {
        if input.keyboard.is_pressed(KeyCode::KeyQ) {
            session.camera_yaw -= dt * 1.6;
        }
        if input.keyboard.is_pressed(KeyCode::KeyE) {
            session.camera_yaw += dt * 1.6;
        }
    }

    if session.phase != Phase::Playing {
        return;
    }

    session.time_played += dt;

    let mut move_dir = Vec3::ZERO;
    if let Some(input) = input {
        let yaw = session.camera_yaw;
        let forward = Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
        let right = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
        if input.keyboard.is_pressed(KeyCode::KeyW) {
            move_dir += forward;
        }
        if input.keyboard.is_pressed(KeyCode::KeyS) {
            move_dir -= forward;
        }
        if input.keyboard.is_pressed(KeyCode::KeyA) {
            move_dir -= right;
        }
        if input.keyboard.is_pressed(KeyCode::KeyD) {
            move_dir += right;
        }
    }
    if move_dir.length_squared() > 0.0 {
        session.dash_dir = move_dir.normalize();
    }

    let dash_pressed = input.is_some_and(|i| {
        i.keyboard.just_pressed(KeyCode::ShiftLeft) || i.keyboard.just_pressed(KeyCode::ShiftRight)
    });
    let mut dashed = false;
    if session.dash_timer > 0.0 {
        session.dash_timer = (session.dash_timer - dt).max(0.0);
    } else if dash_pressed {
        let player = session.player;
        let dash_cost = world
            .get_component::<Ability>(player)
            .filter(|a| a.ready())
            .map(|a| (a.cost_stamina, a.cooldown));
        let has_stamina = world
            .get_component::<AttributeSet>(player)
            .map(|s| s.stamina.current);
        if let (Some((cost, cooldown)), Some(stamina)) = (dash_cost, has_stamina) {
            if stamina >= cost {
                world
                    .get_component_mut::<AttributeSet>(player)
                    .expect("player attrs")
                    .stamina
                    .current -= cost;
                let ability = world
                    .get_component_mut::<Ability>(player)
                    .expect("player ability");
                ability.remaining_cooldown = cooldown;
                session.dash_timer = 0.22;
                dashed = true;
            }
        }
    }
    if dashed {
        let audio = with_audio(world);
        let _ = audio.play_sfx("dash", 1.0);
        audio.mixer.bus_mut(MixerBusId::Sfx).set_volume(1.0);
        audio.sync_mixer();
    }

    let walk_dir = if move_dir.length_squared() > 0.0 {
        move_dir.normalize()
    } else {
        Vec3::ZERO
    };
    let dir = if session.dash_timer > 0.0 {
        session.dash_dir
    } else {
        walk_dir
    };
    let speed = if session.dash_timer > 0.0 { 16.0 } else { 5.5 };
    if dir.length_squared() > 0.0 {
        if let Some(t) = world.get_component_mut::<Transform>(session.player) {
            t.translation += dir * speed * dt;
            t.translation.x = t.translation.x.clamp(-half_bounds(), half_bounds());
            t.translation.z = t.translation.z.clamp(-half_bounds(), half_bounds());
            t.translation.y = if session.dash_timer > 0.0 {
                0.85 + (session.dash_timer * 12.0).sin().abs() * 0.15
            } else {
                0.85
            };
        }
    }

    let player_pos = world
        .get_component::<Transform>(session.player)
        .map(|t| t.translation)
        .unwrap_or(Vec3::ZERO);

    if session.dash_timer > 0.0 {
        session.particles.spawn_from_emitter(
            &mut session.dash_emitter.emitter,
            dt,
            player_pos,
            -session.dash_dir,
        );
    }
    // GPU compute owns integration when the renderer particle system is live.
    session.stream_bytes = session.particles.buffer().upload_bytes().len();

    if let Some(attrs) = world.get_component_mut::<AttributeSet>(session.player) {
        attrs.stamina.current = (attrs.stamina.current + 18.0 * dt).min(attrs.stamina.max);
    }

    // World Partition streams around the player (camera-follow proxy).
    if let Some(report) = tick_streaming(world, player_pos) {
        session.last_stream = report;
    }

    // Guardrail + diagnostics: PCG once spawned props at ~1e8 m; those destroy
    // float precision and flash the whole HDR buffer white via bloom.
    {
        let mut max_r = 0.0_f32;
        let mut far_n = 0u32;
        let q = world.query::<(&StreamedActor, &Transform)>();
        for (actor, t) in q.iter(world) {
            let r = t.translation.length();
            max_r = max_r.max(r);
            if r > 500.0 {
                far_n += 1;
                if far_n == 1 {
                    log::warn!(
                        "streamed actor '{}' kind={} at ({:.1},{:.1},{:.1}) |r|={:.0}",
                        actor.id,
                        actor.kind,
                        t.translation.x,
                        t.translation.y,
                        t.translation.z,
                        r
                    );
                }
            }
        }
        if far_n > 0 && (session.time_played * 2.0) as u32 % 4 == 0 {
            log::warn!(
                "streamed actors beyond 500m: {far_n} (max |r|={max_r:.0}) — regen worlds/arena"
            );
        }
    }

    for (entity, sentinel) in &mut session.sentinels {
        let s_pos = world
            .get_component::<Transform>(*entity)
            .map(|t| t.translation)
            .unwrap_or(Vec3::ZERO);
        let dist = (player_pos - s_pos).length();

        let state_name = if let Some(agent) = world.get_component_mut::<StateTreeComponent>(*entity)
        {
            agent.tick_with_distance(dist, dt);
            agent.tree.active_name().to_string()
        } else {
            "Patrol".to_string()
        };

        match state_name.as_str() {
            "Chase" => {
                let to_player = player_pos - s_pos;
                let flat = Vec3::new(to_player.x, 0.0, to_player.z);
                if flat.length_squared() > 1e-4 {
                    let step = flat.normalize() * 4.2 * dt;
                    if let Some(t) = world.get_component_mut::<Transform>(*entity) {
                        t.translation += step;
                        t.translation.y = 0.9;
                        t.rotation = Quat::from_rotation_y(flat.x.atan2(flat.z));
                    }
                }
            }
            _ => {
                // Patrol — orbital path around hub.
                sentinel.angle += sentinel.speed * dt;
                let offset = Vec3::new(sentinel.angle.cos(), 0.0, sentinel.angle.sin())
                    * sentinel.orbit_radius;
                if let Some(t) = world.get_component_mut::<Transform>(*entity) {
                    t.translation = sentinel.hub + offset + Vec3::new(0.0, 0.9, 0.0);
                    t.rotation =
                        Quat::from_rotation_y(sentinel.angle + std::f32::consts::FRAC_PI_2);
                }
            }
        }

        let s_pos = world
            .get_component::<Transform>(*entity)
            .map(|t| t.translation)
            .unwrap_or(Vec3::ZERO);
        if !GOD_MODE
            && (player_pos - s_pos).length() < 1.45
            && let Some(effects) = world.get_component_mut::<ActiveEffects>(session.player)
        {
            let burning = effects
                .effects
                .iter()
                .any(|e| e.duration > 0.0 && e.health_delta_per_sec < 0.0);
            if !burning {
                effects.effects.push(GameplayEffect::dot(14.0, 0.35));
            }
        }
    }

    for &crystal_e in &session.crystals {
        let spin = world.get_component_mut::<Crystal>(crystal_e).map(|c| {
            c.spin += dt * 1.4;
            c.spin
        });
        if let Some(spin) = spin {
            if let Some(t) = world.get_component_mut::<Transform>(crystal_e) {
                t.rotation = Quat::from_rotation_y(spin);
            }
        }
    }

    let mut picked = Vec::new();
    session.crystals.retain(|&e| {
        let pos = world
            .get_component::<Transform>(e)
            .map(|t| t.translation)
            .unwrap_or(Vec3::ZERO);
        if (player_pos - pos).length() < 1.35 {
            picked.push(pos);
            world.despawn(e);
            false
        } else {
            true
        }
    });
    for pos in picked {
        session.collected += 1;
        session.particles.spawn_from_emitter(
            &mut session.pickup_emitter.emitter,
            0.05,
            pos,
            Vec3::Y,
        );
        if let Some(attrs) = world.get_component_mut::<AttributeSet>(session.player) {
            attrs.health.current = (attrs.health.current + 12.0).min(attrs.health.max);
        }
        let audio = with_audio(world);
        let _ = audio.play_sfx("pickup", 0.85);
        audio
            .mixer
            .bus_mut(MixerBusId::Music)
            .set_volume(0.5 + session.collected as f32 * 0.05);
        audio.sync_mixer();
    }

    if GOD_MODE {
        if let Some(attrs) = world.get_component_mut::<AttributeSet>(session.player) {
            attrs.health.current = attrs.health.max;
        }
    }

    if session.collected >= CRYSTAL_COUNT as u8 {
        session.phase = Phase::Won;
        session.best_time = session.best_time.min(session.time_played);
    } else if !GOD_MODE
        && world
            .get_component::<AttributeSet>(session.player)
            .is_some_and(|a| a.health.current <= 0.0)
    {
        session.phase = Phase::Lost;
    }
}

fn update_camera(world: &World, session: &Session, ctx: &RenderContext) -> (Vec3, Quat, Camera) {
    let target = world
        .get_component::<Transform>(session.player)
        .map(|t| t.translation + Vec3::new(0.0, 0.8, 0.0))
        .unwrap_or(Vec3::Y);
    let dist = 18.0;
    let pitch = 0.95_f32;
    let yaw = session.camera_yaw;
    let offset = Vec3::new(
        yaw.sin() * pitch.cos() * dist,
        pitch.sin() * dist + 4.0,
        yaw.cos() * pitch.cos() * dist,
    );
    let mut cam_pos = target + offset;
    let inner = ARENA * 0.5 - 2.0;
    cam_pos.x = cam_pos.x.clamp(-inner, inner);
    cam_pos.z = cam_pos.z.clamp(-inner, inner);
    let to_target = target - cam_pos;
    let cam_rot = if to_target.length_squared() > 1e-8 {
        Quat::from_rotation_arc(Vec3::NEG_Z, to_target.normalize())
    } else {
        Quat::IDENTITY
    };
    let camera = Camera::new(
        0.82,
        ctx.config.width as f32 / ctx.config.height as f32,
        0.1,
        250.0,
    );
    (cam_pos, cam_rot, camera)
}

fn build_lights(time: f32, vsm: bool) -> LightScene {
    let mut sun = DirectionalLight::new(
        Vec3::new(-0.45, 1.0, 0.25),
        Vec3::new(1.0, 0.94, 0.88),
        1.25,
    );
    // VSM-only when enabled: temporal SMRT screen pass (soft_shadows=2 in LightUniform).
    sun.vsm = vsm;

    let mut points = Vec::new();
    for i in 0..6 {
        let a = i as f32 / 6.0 * TAU + time * 0.08;
        let r = 11.0;
        let mut l = PointLight::new(
            Vec3::new(a.cos() * r, 3.8 + (a * 2.0).sin() * 0.4, a.sin() * r),
            match i % 3 {
                0 => Vec3::new(1.0, 0.35, 0.2),
                1 => Vec3::new(0.25, 0.5, 1.0),
                _ => Vec3::new(0.4, 1.0, 0.55),
            },
            14.0,
            9.0,
        );
        l.cast_shadows = true;
        points.push(l);
    }

    LightScene {
        directional: sun,
        ambient: Vec3::new(0.08, 0.09, 0.12),
        points,
        spots: vec![],
    }
}

fn extract_objects(world: &World, assets: &ArenaAssets, ctx: &RenderContext) -> Vec<RenderObject> {
    let mut out = Vec::new();
    for e in world.all_entities() {
        if let (Some(t), Some(r)) = (
            world.get_component::<Transform>(e),
            world.get_component::<Renderable>(e),
        ) {
            let bounds = ctx
                .get_mesh_bounds(r.mesh_index)
                .map(|b| b.transform(t.matrix()))
                .unwrap_or_else(|| redox_math::Aabb {
                    min: Vec3::splat(-1.0e9),
                    max: Vec3::splat(1.0e9),
                });
            out.push(RenderObject {
                model_matrix: t.matrix(),
                color: [1.0; 4],
                mesh_index: r.mesh_index,
                material_index: r.material_index,
                bounds,
            });
            continue;
        }

        // Streamed OFPA actors (props / walls) — decorative only.
        let Some(actor) = world.get_component::<StreamedActor>(e) else {
            continue;
        };
        let Some(t) = world.get_component::<Transform>(e) else {
            continue;
        };
        let (scale, mat) = match actor.kind.as_str() {
            "wall" => (Vec3::new(2.0, 3.0, 2.0), assets.stream_wall_mat),
            "prop" => (Vec3::new(0.6, 0.9, 0.6), assets.stream_prop_mat),
            _ => continue,
        };
        // Broken PCG offsets put props at 1e7+ m; skip them so MVP math cannot
        // NaN the HDR target when the camera faces that direction.
        if t.translation.length() > 500.0 {
            continue;
        }
        let model = Transform {
            translation: t.translation,
            rotation: t.rotation,
            scale,
        }
        .matrix();
        let bounds = ctx
            .get_mesh_bounds(assets.cube)
            .map(|b| b.transform(model))
            .unwrap_or_else(|| redox_math::Aabb {
                min: Vec3::splat(-1.0e9),
                max: Vec3::splat(1.0e9),
            });
        out.push(RenderObject {
            model_matrix: model,
            color: [1.0; 4],
            mesh_index: assets.cube,
            material_index: mat,
            bounds,
        });
    }
    out
}

fn draw_hud(
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

fn spawn_mesh(world: &mut World, mesh: usize, material: usize, pos: Vec3, scale: Vec3) -> Entity {
    let e = world.spawn();
    world.add_component(
        e,
        Transform {
            translation: pos,
            rotation: Quat::IDENTITY,
            scale,
        },
    );
    world.add_component(
        e,
        Renderable {
            mesh_index: mesh,
            material_index: material,
        },
    );
    e
}

fn half_bounds() -> f32 {
    ARENA * 0.5 - 1.5
}
