//! Courier: a small third-person game that exercises the pieces added in the texture and
//! animation work, together:
//!
//! * assets: everything (level text, textures) is read from a `.pak`; the big ground texture is a
//!   KTX2 file with 12 mips that the `TextureStreamer` brings in small-first;
//! * animation: a procedural humanoid driven by an animation graph (idle/walk/run blend tree,
//!   an arm-only wave layer, root motion decides the speed);
//! * UI: main menu, HUD, pause and win screens with the game UI toolkit, English and Russian
//!   (with plural forms), keyboard and mouse navigation.
//!
//! Keys: WASD / arrows move, Q/E turn the camera, Space waves, Esc pauses, Enter selects.
//! Gamepad: left stick moves, right stick turns the camera, A waves and confirms, D-pad moves the
//! menu focus, B / Start pauses. The best time and the language are saved between runs.
//!
//! Run: `cargo run --example courier -p redox_core --release`
//! Tests (game rules, rig, screens): `cargo test --example courier -p redox_core`

// Demo code: panicking on setup failure is acceptable here.
#![allow(clippy::unwrap_used)]

mod game;
mod pad;
mod progress;
mod rig;
mod settings;
mod settings_menu;
mod sounds;
mod ui;
mod world_assets;

use std::sync::Arc;
use std::time::Instant;

use redox_anim::Pose;
use redox_asset::{
    AssetManager, AssetSource, Handle, Ktx2Texture, PakSource, Priority, StreamedTexture,
};
use redox_audio::{AudioContext, CueLibrary, CueSelector};
use redox_core::config::EngineConfig;
use redox_core::window::{RunClosure, create_window};
use redox_ecs::World as EcsWorld;
use redox_input::{GamepadsState, GilrsBackend, InputState};
use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::asset_types::MaterialData;
use redox_render::camera::Camera;
use redox_render::context::RenderContext;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive;
use redox_render::resource::texture_stream::{StreamConfig, TextureStreamer};
use redox_render::{RenderObject, Renderer, SkinnedMeshBinding};
use redox_ui::UiContext;
use redox_ui::game::paint::{egui_measure, paint};
use redox_ui::game::{Direction, Focus, ScreenStack, UiScale, layout_localized};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

use game::{Controls, World};
use redox_scene::save::SaveManager;
use settings::Settings;
use settings_menu::{MenuAction, SettingsMenu};

/// Everything the renderer needs to draw the scene.
struct Scene {
    cube: usize,
    coin_mesh: usize,
    solid_ground: usize,
    solid_crate: usize,
    coin_material: usize,
    hero_material: usize,
    ground: Streamed,
    crate_tex: Streamed,
    hero: SkinnedMeshBinding,
}

/// A streamed texture and the material that uses it once it exists.
struct Streamed {
    handle: Handle<StreamedTexture>,
    textured: Option<usize>,
}

impl Streamed {
    fn material(&self, fallback: usize) -> usize {
        self.textured.unwrap_or(fallback)
    }
}

fn light_scene() -> LightScene {
    let mut sun =
        DirectionalLight::new(Vec3::new(-0.45, 1.0, 0.55), Vec3::new(1.0, 0.94, 0.84), 2.4);
    sun.soft_shadows = false;
    sun.vsm = true;
    LightScene {
        directional: sun,
        ambient: Vec3::new(0.12, 0.14, 0.18),
        points: vec![],
        spots: vec![],
    }
}

fn load_environment(r: &mut Renderer) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/skybox.hdr");
    let Ok(bytes) = std::fs::read(&path) else {
        return;
    };
    if let Ok(texture) = redox_render::resource::texture::Texture::from_hdr_bytes(
        &r.device, &r.queue, &bytes, "skybox",
    ) {
        r.set_environment(&texture);
        r.set_skybox(true);
    }
}

fn model_matrix(pos: Vec3, yaw: f32) -> Mat4 {
    Mat4::from_rotation_translation(Quat::from_rotation_y(yaw), pos)
}

fn bounds_of(r: &Renderer, mesh: usize, model: Mat4) -> Aabb {
    r.get_mesh_bounds(mesh).map_or(
        Aabb {
            min: Vec3::splat(-1.0e9),
            max: Vec3::splat(1.0e9),
        },
        |b| b.transform(model),
    )
}

fn object(r: &Renderer, model: Mat4, mesh: usize, material: usize) -> RenderObject {
    RenderObject {
        model_matrix: model,
        color: [1.0; 4],
        mesh_index: mesh,
        material_index: material,
        bounds: bounds_of(r, mesh, model),
    }
}

impl Scene {
    fn new(
        r: &mut Renderer,
        assets: &mut AssetManager,
        streamer: &mut TextureStreamer,
        hero: &redox_anim::SkinnedModel,
    ) -> Self {
        let cube = r.upload_mesh(&primitive::create_cube());
        let coin_mesh = r.upload_mesh(&primitive::create_sphere(0.25, 18));
        let solid_ground = r.add_material(Material::solid(Vec3::new(0.30, 0.42, 0.28)));
        let solid_crate = r.add_material(Material::solid(Vec3::new(0.55, 0.38, 0.22)));
        let mut gold = Material::solid(Vec3::new(1.0, 0.75, 0.2));
        gold.metallic = 0.9;
        gold.roughness = 0.25;
        gold.emissive = Vec3::new(1.2, 0.8, 0.15);
        let coin_material = r.add_material(gold);
        let mut skin = Material::solid(Vec3::new(0.95, 0.45, 0.2));
        skin.roughness = 0.5;
        let hero_material = r.add_material(skin);
        let bind = hero
            .geometry
            .positions
            .iter()
            .fold(Aabb::empty(), |b, p| b.expand(Vec3::from(*p)));
        let hero =
            SkinnedMeshBinding::new(r, &hero.geometry, 2.0 * bind.half_extents().max_element());
        let ground = streamer
            .add(assets, world_assets::GROUND, Priority::HIGH)
            .unwrap();
        let crate_tex = streamer
            .add(assets, world_assets::CRATE, Priority::NORMAL)
            .unwrap();
        Self {
            cube,
            coin_mesh,
            solid_ground,
            solid_crate,
            coin_material,
            hero_material,
            ground: Streamed {
                handle: ground,
                textured: None,
            },
            crate_tex: Streamed {
                handle: crate_tex,
                textured: None,
            },
            hero,
        }
    }

    /// Gives textures that just became ready their material.
    fn bind_streamed(
        &mut self,
        r: &mut Renderer,
        assets: &mut AssetManager,
        streamer: &TextureStreamer,
    ) {
        for streamed in [&mut self.ground, &mut self.crate_tex] {
            if streamed.textured.is_none() && streamer.slot_of(streamed.handle).is_some() {
                let texture = Handle::<Ktx2Texture>::new(streamed.handle.id());
                let data = assets.insert(MaterialData::solid(Vec3::ONE).with_albedo_ktx2(texture));
                streamed.textured = r.add_material_from_asset(data, assets.get(data).unwrap());
            }
        }
    }

    /// Objects to draw plus a stable id per object (ground 1, hero 2, crates 100+, coins 1000+),
    /// so the renderer follows each object's motion even when coins vanish from the list.
    fn objects(&self, r: &Renderer, world: &World, time: f32) -> (Vec<RenderObject>, Vec<u64>) {
        let half = world.level.half;
        let mut out = Vec::new();
        let mut ids: Vec<u64> = Vec::new();
        let ground = Mat4::from_scale_rotation_translation(
            Vec3::new(half * 2.0, 0.25, half * 2.0),
            Quat::IDENTITY,
            Vec3::new(0.0, -0.125, 0.0),
        );
        out.push(object(
            r,
            ground,
            self.cube,
            self.ground.material(self.solid_ground),
        ));
        ids.push(1);
        for (i, c) in world.level.crates.iter().enumerate() {
            let model = Mat4::from_scale_rotation_translation(
                Vec3::splat(c.half * 2.0),
                Quat::IDENTITY,
                Vec3::new(c.x, c.half, c.z),
            );
            out.push(object(
                r,
                model,
                self.cube,
                self.crate_tex.material(self.solid_crate),
            ));
            ids.push(100 + i as u64);
        }
        for (i, ((x, z), taken)) in world.level.coins.iter().zip(&world.taken).enumerate() {
            if *taken {
                continue;
            }
            let bob = 0.65 + 0.12 * (time * 2.5 + i as f32).sin();
            let model = Mat4::from_scale_rotation_translation(
                Vec3::new(1.0, 1.0, 0.25),
                Quat::from_rotation_y(time * 2.0 + i as f32),
                Vec3::new(*x, bob, *z),
            );
            out.push(object(r, model, self.coin_mesh, self.coin_material));
            ids.push(1000 + i as u64);
        }
        let hero = model_matrix(world.pos, world.yaw);
        out.push(RenderObject {
            model_matrix: hero,
            color: [1.0; 4],
            mesh_index: self.hero.mesh_slot,
            material_index: self.hero_material,
            bounds: self.hero.bounds.transform(hero),
        });
        ids.push(2);
        (out, ids)
    }
}

fn camera_pose(world: &World, orbit: f32) -> (Vec3, Quat) {
    let focus = world.pos + Vec3::Y * 1.2;
    let yaw = world.camera_yaw + orbit;
    let behind = Vec3::new(yaw.sin(), 0.0, yaw.cos());
    let pos = focus - behind * 5.2 + Vec3::Y * 2.2;
    (
        pos,
        Quat::from_rotation_arc(Vec3::NEG_Z, (focus - pos).normalize()),
    )
}

fn controls(input: &InputState, wave: bool) -> Controls {
    let key = |codes: &[KeyCode]| codes.iter().any(|c| input.keyboard.is_pressed(*c));
    let axis =
        |plus: &[KeyCode], minus: &[KeyCode]| f32::from(i8::from(key(plus)) - i8::from(key(minus)));
    Controls {
        move_x: axis(
            &[KeyCode::KeyD, KeyCode::ArrowRight],
            &[KeyCode::KeyA, KeyCode::ArrowLeft],
        ),
        move_z: axis(
            &[KeyCode::KeyW, KeyCode::ArrowUp],
            &[KeyCode::KeyS, KeyCode::ArrowDown],
        ),
        wave,
        camera_turn: axis(&[KeyCode::KeyE], &[KeyCode::KeyQ]),
    }
}

/// Pushes the settings to the renderer and the mixer.
fn apply_settings(settings: &Settings, ctx: &mut RenderContext, audio: &mut AudioContext) {
    audio.set_master_volume(f64::from(settings.master_volume));
    ctx.bloom_enabled = settings.bloom;
    ctx.taa_enabled = settings.taa;
    ctx.set_ssao_enabled(settings.ssao);
    ctx.set_vsm_enabled(settings.shadows);
}

/// Applies the settings and writes them to disk.
fn apply_and_save(
    settings: &Settings,
    saves: &SaveManager,
    ctx: &mut RenderContext,
    audio: &mut AudioContext,
) {
    apply_settings(settings, ctx, audio);
    settings.save(saves);
}

/// A button was chosen; the game decides what that means.
fn act(
    button: &str,
    screens: &mut ScreenStack,
    loc: &mut redox_ui::game::Localization,
    restart: &mut bool,
) -> bool {
    match button {
        "play" => {
            *restart = true;
            screens.reset("hud", false);
        }
        "language" => {
            let next = if loc.language() == "en" { "ru" } else { "en" };
            loc.set_language(next);
        }
        "settings" => screens.push("settings", true),
        "resume" => {
            screens.pop();
        }
        "menu" => screens.reset("menu", true),
        "quit" => return true,
        _ => {}
    }
    false
}

fn main() {
    let pak_path = std::env::temp_dir().join("courier_assets.pak");
    world_assets::build_pak(&pak_path).expect("write the asset pack");
    let source: Arc<dyn AssetSource> = Arc::new(PakSource::open(&pak_path).expect("open the pack"));
    let level_text =
        String::from_utf8(source.read(world_assets::LEVEL).expect("level in the pack")).unwrap();
    let level = game::parse_level(&level_text).expect("valid level");

    let config = EngineConfig {
        window_title: "RedOx — Courier".to_string(),
        ..Default::default()
    };
    let (event_loop, window) = create_window(&config);
    let mut ctx = pollster::block_on(RenderContext::new(
        window.clone(),
        config.wgpu_present_mode(),
    ));
    ctx.set_render_scale(config.render_scale);
    ctx.set_ssao_enabled(true);
    ctx.bloom_enabled = true;
    ctx.set_exposure(0.9);
    ctx.set_ibl_intensity(0.55);
    load_environment(&mut ctx);

    let mut assets = AssetManager::with_workers(".", 2);
    assets.set_source(source);
    let mut ecs = EcsWorld::new();
    let mut streamer = TextureStreamer::new(StreamConfig {
        texture_budget_bytes: 64 << 20,
        demote_after_frames: 90,
        ..Default::default()
    });
    let model = rig::humanoid();
    let mut scene = Scene::new(&mut ctx, &mut assets, &mut streamer, &model);
    let mut world = World::new(level.clone(), &model.skeleton);

    let mut ui_ctx = UiContext::new(&window, ctx.device(), ctx.surface_format());
    ui_ctx.show_debug = false;
    let mut loc = ui::localization();
    let saves = progress::save_manager();
    let mut progress = progress::Progress::load(&saves);
    if let Some(language) = &progress.language {
        loc.set_language(language);
    }
    let mut audio = AudioContext::new();
    sounds::register_all(&mut audio);
    let mut settings = Settings::load(&saves);
    apply_settings(&settings, &mut ctx, &mut audio);
    let mut settings_menu = SettingsMenu::new();
    let cues = CueLibrary::parse(sounds::CUES).expect("valid cue table");
    let mut selector = CueSelector::new();
    let mut pads = GamepadsState::new();
    let mut pad_backend = GilrsBackend::new();
    let mut pad_move = (0.0f32, 0.0f32, 0.0f32);
    let mut new_record = false;
    let mut screens = ScreenStack::default();
    screens.reset("menu", true);
    let mut focus = Focus::default();
    let mut input = InputState::new();
    let (mut cursor, mut click, mut wave_pressed) = ((0.0f32, 0.0f32), false, false);
    let mut activate = false;
    let mut nav: Option<Direction> = None;
    let mut escape = false;
    let mut last = Instant::now();
    let start = Instant::now();
    let mut orbit = 0.0f32;

    event_loop
        .run_closure(move |event, elwt| match event {
            winit::event::Event::NewEvents(_) => {
                input.begin_frame();
                pads.begin_frame();
                if let Some(backend) = pad_backend.as_mut() {
                    backend.poll(&mut pads);
                }
                let pad = pad::read(&pads);
                escape |= pad.back;
                activate |= pad.confirm;
                wave_pressed |= pad.wave;
                nav = nav.or(pad.nav);
                pad_move = (pad.move_x, pad.move_z, pad.camera_turn);
            }
            winit::event::Event::WindowEvent { event, .. } => {
                let consumed = ui_ctx.handle_window_event(&window, &event);
                input.process_window_event(&event);
                match event {
                    WindowEvent::KeyboardInput { event: key, .. }
                        if key.state == ElementState::Pressed && !key.repeat =>
                    {
                        if let PhysicalKey::Code(code) = key.physical_key {
                            match code {
                                KeyCode::Escape => escape = true,
                                KeyCode::Enter | KeyCode::NumpadEnter => activate = true,
                                KeyCode::Space => {
                                    wave_pressed = true;
                                    activate = true;
                                }
                                KeyCode::ArrowDown | KeyCode::KeyS => nav = Some(Direction::Down),
                                KeyCode::ArrowUp | KeyCode::KeyW => nav = Some(Direction::Up),
                                KeyCode::ArrowLeft | KeyCode::KeyA => nav = Some(Direction::Left),
                                KeyCode::ArrowRight | KeyCode::KeyD => {
                                    nav = Some(Direction::Right);
                                }
                                KeyCode::Tab => nav = Some(Direction::Next),
                                KeyCode::F1 | KeyCode::F2 | KeyCode::F3 | KeyCode::F4 => {
                                    // Debug switches for hunting rendering artefacts.
                                    match code {
                                        KeyCode::F1 => ctx.bloom_enabled = !ctx.bloom_enabled,
                                        KeyCode::F2 => ctx.taa_enabled = !ctx.taa_enabled,
                                        KeyCode::F3 => {
                                            let on = !ctx.ssao_enabled;
                                            ctx.set_ssao_enabled(on);
                                        }
                                        _ => {
                                            let on = !ctx.vsm_enabled;
                                            ctx.set_vsm_enabled(on);
                                        }
                                    }
                                    // The saved settings follow the switches, so the screen agrees.
                                    settings.bloom = ctx.bloom_enabled;
                                    settings.taa = ctx.taa_enabled;
                                    settings.ssao = ctx.ssao_enabled;
                                    settings.shadows = ctx.vsm_enabled;
                                    settings.save(&saves);
                                    let flag = |on: bool| if on { "on" } else { "off" };
                                    window.set_title(&format!(
                                        "RedOx - Courier | F1 bloom {} | F2 TAA {} | F3 SSAO {} | F4 shadows {}",
                                        flag(ctx.bloom_enabled),
                                        flag(ctx.taa_enabled),
                                        flag(ctx.ssao_enabled),
                                        flag(ctx.vsm_enabled)
                                    ));
                                }
                                _ => {}
                            }
                        }
                    }
                    WindowEvent::CursorMoved { position, .. } if !consumed => {
                        cursor = (position.x as f32, position.y as f32);
                    }
                    WindowEvent::MouseInput {
                        state: ElementState::Pressed,
                        button: MouseButton::Left,
                        ..
                    } => click = true,
                    WindowEvent::CloseRequested => elwt.exit(),
                    WindowEvent::Resized(size) => ctx.resize(size.width, size.height),
                    WindowEvent::RedrawRequested => {
                        let dt = last.elapsed().as_secs_f32().min(0.05);
                        last = Instant::now();
                        let time = start.elapsed().as_secs_f32();
                        let top = screens.top().map(|s| s.name.clone()).unwrap_or_default();

                        // Escape: pause / resume / back.
                        if std::mem::take(&mut escape) {
                            match top.as_str() {
                                "hud" => screens.push("pause", false),
                                "pause" | "settings" => {
                                    screens.pop();
                                }
                                "win" => screens.reset("menu", true),
                                _ => {}
                            }
                        }
                        let top = screens.top().map(|s| s.name.clone()).unwrap_or_default();

                        // Simulation: only while playing; the menu shows the idle hero and a slow orbit.
                        let playing = top == "hud";
                        let want = if playing {
                            let mut want = controls(&input, std::mem::take(&mut wave_pressed));
                            want.move_x = (want.move_x + pad_move.0).clamp(-1.0, 1.0);
                            want.move_z = (want.move_z + pad_move.1).clamp(-1.0, 1.0);
                            want.camera_turn = (want.camera_turn + pad_move.2).clamp(-1.0, 1.0);
                            want
                        } else {
                            wave_pressed = false;
                            Controls::default()
                        };
                        if want.wave {
                            audio.play_cue(&cues, &mut selector, "wave", time);
                        }
                        if !matches!(top.as_str(), "pause" | "win" | "settings") {
                            let step = world.step(dt, want, &model.skeleton, &model.clips);
                            if step.picked > 0 {
                                log::info!("picked up {} coin(s)", step.picked);
                                audio.play_cue(&cues, &mut selector, "coin", time);
                            }
                            let skin = Pose::from_local(&model.skeleton, &step.output.pose)
                                .skin_matrices(&model.skeleton);
                            if !scene.hero.update(&ctx, &skin) {
                                log::warn!("skinned mesh update rejected");
                            }
                        }
                        orbit = if playing { 0.0 } else { orbit + 0.25 * dt };
                        if playing && let Some(seconds) = world.finished {
                            new_record = progress.record_run(seconds);
                            if new_record {
                                progress.save(&saves);
                            }
                            audio.play_cue(&cues, &mut selector, "win", time);
                            screens.push("win", false);
                        }

                        assets.update(&mut ecs);
                        scene.bind_streamed(&mut ctx, &mut assets, &streamer);
                        let hero_bounds = scene
                            .hero
                            .swept_bounds()
                            .transform(model_matrix(world.pos, world.yaw));
                        ctx.invalidate_shadow_region(&[hero_bounds]);

                        let camera = Camera::new(
                            0.9,
                            ctx.config.width as f32 / ctx.config.height as f32,
                            0.1,
                            120.0,
                        );
                        let (pos, rot) = camera_pose(&world, orbit);
                        ctx.camera_uniform.update(&camera, pos, rot);
                        ctx.update_camera_buffer();
                        let (objects, object_ids) = scene.objects(&ctx, &world, time);
                        streamer.update_from_scene(&ctx, &objects);
                        streamer.update(&mut ctx, &mut assets);

                        let lights = light_scene();
                        let fv = FrameView::new(&camera, &objects, &lights)
                            .with_object_ids(&object_ids);
                        let output = match redox_render::acquire_surface_texture(ctx.surface()) {
                            Ok(output) => output,
                            Err(
                                redox_render::SurfaceError::Outdated
                                | redox_render::SurfaceError::Lost,
                            ) => {
                                let (w, h) = (ctx.config.width, ctx.config.height);
                                ctx.resize(w, h);
                                return;
                            }
                            Err(
                                redox_render::SurfaceError::Timeout
                                | redox_render::SurfaceError::Occluded,
                            ) => return,
                            Err(redox_render::SurfaceError::Validation) => {
                                elwt.exit();
                                return;
                            }
                        };
                        let view = output.texture.create_view(&Default::default());
                        ctx.render_frame_view(&fv, &view).unwrap();

                        // ── UI ──
                        let mut encoder =
                            ctx.device()
                                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                    label: Some("courier_ui"),
                                });
                        ui_ctx.begin_frame(&window);
                        let points = ui_ctx.egui_ctx.content_rect();
                        let scale = UiScale::new(points.width(), points.height());
                        let painter = ui_ctx.egui_ctx.layer_painter(egui::LayerId::new(
                            egui::Order::Foreground,
                            egui::Id::new("courier_ui"),
                        ));
                        let measure = egui_measure(&painter, scale);
                        let root = scale.root(0.03);
                        let ground_base = streamer.resident_base(scene.ground.handle).unwrap_or(0);
                        let stats = streamer.stats();
                        let hud = ui::HudData {
                            coins: world.coins_taken(),
                            total: world.taken.len(),
                            seconds: world.finished.unwrap_or(world.elapsed),
                            state: format!(
                                "{}{}",
                                world.animation_state(),
                                if world.waving() { " + wave" } else { "" }
                            ),
                            mip: ground_base,
                            resident_kib: stats.resident_bytes / 1024,
                        };
                        let mut quit = false;
                        let visible: Vec<String> =
                            screens.visible().iter().map(|s| s.name.clone()).collect();
                        for name in visible {
                            let tree = match name.as_str() {
                                "menu" => ui::main_menu(progress.best_seconds),
                                "settings" => ui::settings_menu(
                                    &settings_menu,
                                    &settings,
                                    loc.language() == "ru",
                                ),
                                "pause" => ui::pause_menu(),
                                "win" => ui::win_screen(
                                    world.finished.unwrap_or(0.0),
                                    progress.best_seconds,
                                    new_record,
                                ),
                                _ => ui::hud(&hud, &loc),
                            };
                            let laid = layout_localized(&tree, root, &measure, &loc);
                            let is_top = screens.top().is_some_and(|t| t.name == name);
                            if is_top && name == "settings" {
                                if let Some(direction) = nav.take() {
                                    match direction {
                                        Direction::Up | Direction::Previous => {
                                            settings_menu.move_selection(-1);
                                        }
                                        Direction::Down | Direction::Next => {
                                            settings_menu.move_selection(1);
                                        }
                                        Direction::Left | Direction::Right => {
                                            let step = if direction == Direction::Left { -1 } else { 1 };
                                            if settings_menu.adjust(&mut settings, step) {
                                                apply_and_save(
                                                    &settings, &saves, &mut ctx, &mut audio,
                                                );
                                            }
                                        }
                                    }
                                    audio.play_cue(&cues, &mut selector, "select", time);
                                }
                                let (vx, vy) = scale.to_virtual(
                                    cursor.0 / window.scale_factor() as f32,
                                    cursor.1 / window.scale_factor() as f32,
                                );
                                let hovered = laid.button_at(vx, vy).and_then(|id| ui::row_index(&id));
                                if let Some(index) = hovered {
                                    let delta = index as i32 - settings_menu.selected() as i32;
                                    settings_menu.move_selection(delta);
                                }
                                let clicked = std::mem::take(&mut click) && hovered.is_some();
                                if std::mem::take(&mut activate) || clicked {
                                    match settings_menu.activate(&mut settings) {
                                        MenuAction::Changed => {
                                            apply_and_save(&settings, &saves, &mut ctx, &mut audio);
                                        }
                                        MenuAction::Back => {
                                            screens.pop();
                                        }
                                        MenuAction::None => {}
                                    }
                                    audio.play_cue(&cues, &mut selector, "select", time);
                                }
                                focus.set(&ui::row_button_id(settings_menu.selected()));
                            } else if is_top && name != "hud" {
                                let buttons = laid.buttons();
                                focus.ensure(&buttons);
                                if let Some(direction) = nav.take() {
                                    focus.navigate(direction, &buttons);
                                    audio.play_cue(&cues, &mut selector, "select", time);
                                }
                                let (vx, vy) = scale.to_virtual(
                                    cursor.0 / window.scale_factor() as f32,
                                    cursor.1 / window.scale_factor() as f32,
                                );
                                let hovered = laid.button_at(vx, vy);
                                if let Some(id) = &hovered {
                                    focus.set(id);
                                }
                                let chosen = if std::mem::take(&mut click) {
                                    hovered
                                } else {
                                    None
                                }
                                .or_else(|| {
                                    std::mem::take(&mut activate)
                                        .then(|| focus.current().map(str::to_string))
                                        .flatten()
                                });
                                if let Some(button) = chosen {
                                    audio.play_cue(&cues, &mut selector, "select", time);
                                    let mut restart = false;
                                    quit |= act(&button, &mut screens, &mut loc, &mut restart);
                                    if button == "language" {
                                        progress.language = Some(loc.language().to_string());
                                        progress.save(&saves);
                                    }
                                    if button == "settings" {
                                        settings_menu = SettingsMenu::new();
                                    }
                                    if restart {
                                        world = World::new(level.clone(), &model.skeleton);
                                    }
                                }
                            }
                            paint(
                                &laid,
                                &painter,
                                scale,
                                if is_top { focus.current() } else { None },
                            );
                        }
                        nav = None;
                        activate = false;
                        click = false;
                        drop(measure);
                        ui_ctx.end_frame_and_render(
                            ctx.device(),
                            ctx.queue(),
                            &mut encoder,
                            &window,
                            &view,
                            egui_wgpu::ScreenDescriptor {
                                size_in_pixels: [ctx.config.width, ctx.config.height],
                                pixels_per_point: window.scale_factor() as f32,
                            },
                        );
                        ctx.queue().submit(std::iter::once(encoder.finish()));
                        ctx.queue().present(output);
                        if quit {
                            elwt.exit();
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
