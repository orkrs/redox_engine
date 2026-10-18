//! Foxfire: a small action game on the RedOx engine.
//!
//! A fox (rigged glTF model with Idle / Walk / Run clips, blended by an animation graph) runs
//! through a forest glade collecting glowing gems before the lantern burns out. Sprinting costs
//! stamina, dark wisps take time away, picking gems in a row builds a combo.
//!
//! Keys: WASD / arrows move, Shift sprints, Q/E turn the camera, Esc pauses, Enter selects.
//! Gamepad: left stick moves, right stick turns the camera, right trigger / bumper sprints,
//! D-pad moves the menu focus, A selects, B / Start pauses.
//!
//! Run: `cargo run --example foxrun -p redox_core --release`
//! Screenshot without a window: `... --example foxrun -p redox_core --release -- --shot out.png`
//! Tests (rules, model, screens): `cargo test --example foxrun -p redox_core`

// Demo code: panicking on setup failure is acceptable here.
#![allow(clippy::unwrap_used)]

mod fox;
mod save;
mod scene;
mod sounds;
mod ui;
mod world;

#[allow(dead_code)]
#[path = "../courier/pad.rs"]
mod pad;

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use redox_anim::graph::GraphPlayer;
use redox_audio::{AudioContext, CueLibrary, CueSelector};
use redox_core::config::EngineConfig;
use redox_core::window::{RunClosure, create_window};
use redox_input::{GamepadButton, GamepadsState, GilrsBackend, InputState};
use redox_math::{Quat, Vec3};
use redox_render::Renderer;
use redox_render::camera::Camera;
use redox_render::context::RenderContext;
use redox_render::frame::FrameView;
use redox_render::resource::ibl::read_texture_ktx2;
use redox_ui::UiContext;
use redox_ui::game::paint::{egui_measure, paint};
use redox_ui::game::{Direction, Focus, ScreenStack, UiScale, layout_localized};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

use fox::FoxAssets;
use scene::Scene;
use world::{Controls, Events, Phase, RUN_SPEED, World};

const FIELD_OF_VIEW: f32 = 0.95;

fn asset_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/foxrun")
        .join(name)
}

fn load_environment(r: &mut Renderer) {
    let Ok(bytes) = std::fs::read(asset_path("sky.hdr")) else {
        log::warn!("sky.hdr is missing, the sky stays plain");
        return;
    };
    if let Ok(texture) =
        redox_render::resource::texture::Texture::from_hdr_bytes(&r.device, &r.queue, &bytes, "sky")
    {
        r.set_environment(&texture);
        r.set_skybox(true);
    }
}

fn tune(r: &mut Renderer) {
    r.set_ssao_enabled(true);
    r.bloom_enabled = true;
    r.set_exposure(0.95);
    r.set_ibl_intensity(0.9);
    load_environment(r);
}

/// The world, the fox's animation and the chase camera.
struct Game {
    world: World,
    graph: GraphPlayer,
    cam_pos: Vec3,
    cam_target: Vec3,
}

impl Game {
    fn new(seed: u64, fox: &FoxAssets) -> Self {
        let world = World::new(seed);
        let (eye, target) = scene::camera_target(&world, 0.0);
        Self {
            world,
            graph: GraphPlayer::new(fox.graph(), &fox.model.skeleton),
            cam_pos: eye,
            cam_target: target,
        }
    }

    /// Advances the rules and the animation; returns what happened and the pose to draw.
    fn tick(
        &mut self,
        dt: f32,
        controls: Controls,
        orbit: f32,
        fox: &FoxAssets,
    ) -> (Events, redox_anim::LocalPose) {
        let events = self.world.step(dt, controls);
        self.graph
            .params
            .set_float("speed", (self.world.speed / RUN_SPEED).clamp(0.0, 1.0));
        let output = self.graph.update(dt, &fox.model.skeleton, &fox.model.clips);

        let (eye, target) = scene::camera_target(&self.world, orbit);
        let eye = scene::keep_clear(&self.world, eye);
        let k = 1.0 - (-7.0 * dt.min(0.1)).exp();
        self.cam_pos = self.cam_pos.lerp(eye, k);
        self.cam_target = self.cam_target.lerp(target, k);
        (events, output.pose)
    }

    fn camera_pose(&self) -> (Vec3, Quat) {
        (self.cam_pos, scene::look_at(self.cam_pos, self.cam_target))
    }
}

/// Steers towards the nearest gem and around obstacles; drives the screenshot mode and the tests.
/// When it stops making progress it slides along the nearest obstacle for a moment.
struct Autopilot {
    check_pos: (f32, f32),
    check_timer: f32,
    escape: f32,
    side: f32,
}

impl Autopilot {
    fn new() -> Self {
        Self {
            check_pos: (0.0, 0.0),
            check_timer: 0.0,
            escape: 0.0,
            side: 1.0,
        }
    }

    fn drive(&mut self, world: &World, dt: f32) -> Controls {
        self.check_timer += dt;
        self.escape = (self.escape - dt).max(0.0);
        if self.check_timer >= 0.8 {
            let moved = (world.pos.0 - self.check_pos.0).hypot(world.pos.1 - self.check_pos.1);
            if moved < 0.6 {
                self.side = -self.side;
                self.escape = 1.4;
            }
            self.check_pos = world.pos;
            self.check_timer = 0.0;
        }
        let (px, pz) = world.pos;
        let Some(target) = world.gems.iter().filter(|g| !g.taken).min_by(|a, b| {
            (a.x - px)
                .hypot(a.z - pz)
                .total_cmp(&(b.x - px).hypot(b.z - pz))
        }) else {
            return Controls::default();
        };
        let (mut wx, mut wz) = (target.x - px, target.z - pz);
        let length = wx.hypot(wz).max(1e-3);
        wx /= length;
        wz /= length;
        let nearest = world.obstacles.iter().min_by(|a, b| {
            ((a.x - px).hypot(a.z - pz) - a.radius)
                .total_cmp(&((b.x - px).hypot(b.z - pz) - b.radius))
        });
        if let Some(o) = nearest {
            let (dx, dz) = (px - o.x, pz - o.z);
            let d = dx.hypot(dz).max(1e-3);
            let reach = o.radius + 2.4;
            if self.escape > 0.0 && d < reach + 2.0 {
                // Slide around the obstacle in the chosen direction.
                (wx, wz) = (
                    -dz / d * self.side + dx / d * 0.3,
                    dx / d * self.side + dz / d * 0.3,
                );
            } else if d < reach {
                let push = (reach - d) / reach;
                let side = if wx * dz - wz * dx >= 0.0 { 1.0 } else { -1.0 };
                wx += (dx / d) * push * 1.2 + (-dz / d) * side * push * 1.6;
                wz += (dz / d) * push * 1.2 + (dx / d) * side * push * 1.6;
            }
        }
        let (fx, fz) = world.camera_forward();
        let length = wx.hypot(wz).max(1e-3);
        let (wx, wz) = (wx / length, wz / length);
        Controls {
            move_x: wx * -fz + wz * fx,
            move_z: wx * fx + wz * fz,
            sprint: world.stamina > 0.4 || world.sprinting,
            camera_turn: 0.0,
        }
    }
}

fn keyboard_controls(input: &InputState, pads: &GamepadsState, pad: &pad::PadInput) -> Controls {
    let key = |codes: &[KeyCode]| codes.iter().any(|c| input.keyboard.is_pressed(*c));
    let axis =
        |plus: &[KeyCode], minus: &[KeyCode]| f32::from(i8::from(key(plus)) - i8::from(key(minus)));
    let sprint = key(&[KeyCode::ShiftLeft, KeyCode::ShiftRight])
        || pads.any_pressed(GamepadButton::RightTrigger)
        || pads.any_pressed(GamepadButton::RightShoulder)
        || pads.any_pressed(GamepadButton::LeftShoulder);
    Controls {
        move_x: (axis(
            &[KeyCode::KeyD, KeyCode::ArrowRight],
            &[KeyCode::KeyA, KeyCode::ArrowLeft],
        ) + pad.move_x)
            .clamp(-1.0, 1.0),
        move_z: (axis(
            &[KeyCode::KeyW, KeyCode::ArrowUp],
            &[KeyCode::KeyS, KeyCode::ArrowDown],
        ) + pad.move_z)
            .clamp(-1.0, 1.0),
        sprint,
        camera_turn: (axis(&[KeyCode::KeyE], &[KeyCode::KeyQ]) + pad.camera_turn).clamp(-1.0, 1.0),
    }
}

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
        "resume" => {
            screens.pop();
        }
        "menu" => screens.reset("menu", true),
        "quit" => return true,
        _ => {}
    }
    false
}

fn new_seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(2024, |d| d.as_nanos() as u64)
}

fn load_fox() -> FoxAssets {
    let bytes = std::fs::read(asset_path("Fox.glb")).expect("assets/foxrun/Fox.glb");
    fox::load(&bytes).expect("the fox model")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--shot") {
        let out = args.get(i + 1).map_or("foxrun.png", String::as_str);
        let seconds = args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(14.0);
        let gem_dist = args
            .iter()
            .position(|a| a == "--gem-dist")
            .and_then(|i| args.get(i + 1)?.parse().ok());
        screenshot(out, seconds, gem_dist);
        return;
    }
    play();
}

/// Renders a few seconds of the autopilot to a PNG without opening a window.
fn screenshot(path: &str, seconds: f32, gem_dist: Option<f32>) {
    const W: u32 = 1280;
    const H: u32 = 720;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("a GPU adapter");
    let wanted = wgpu::Features::PRIMITIVE_INDEX
        | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
        | wgpu::Features::TEXTURE_COMPRESSION_BC
        | wgpu::Features::PIPELINE_CACHE
        | redox_render::profiler::PROFILER_FEATURES;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features() & wanted,
        ..Default::default()
    }))
    .expect("a device");
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: wgpu::TextureFormat::Rgba8Unorm,
        width: W,
        height: H,
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
        color_space: Default::default(),
    };
    let mut r = Renderer::from_shared_device(device, queue, &adapter, None, config);
    tune(&mut r);
    let fox = load_fox();
    let mut scene = Scene::new(&mut r, &fox).expect("scene");
    let mut game = Game::new(2024, &fox);
    let mut pilot = Autopilot::new();
    if let Some(distance) = gem_dist {
        // Lighting check: stand `distance` metres in front of the first gem (south of it), facing it.
        let gem = game.world.gems[0];
        game.world.pos = (gem.x, gem.z - distance);
        game.world.yaw = 0.0;
        game.world.wisps.clear();
        game.world
            .obstacles
            .retain(|o| (o.x - gem.x).hypot(o.z - gem.z) > 12.0);
        let (eye, target) = scene::camera_target(&game.world, 0.0);
        game.cam_pos = eye;
        game.cam_target = target;
    }
    let target = r.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shot"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let camera = Camera::new(FIELD_OF_VIEW, W as f32 / H as f32, 0.1, 160.0);
    let dt = 1.0 / 60.0;
    let frames = (seconds / dt) as u32;
    for frame_index in 0..frames + 20 {
        let controls = if frame_index < frames && gem_dist.is_none() {
            pilot.drive(&game.world, dt)
        } else {
            Controls::default()
        };
        let (_, pose) = game.tick(dt, controls, 0.0, &fox);
        scene.pose_fox(&r, &fox, &pose);
        let (pos, rot) = game.camera_pose();
        r.camera_uniform.update(&camera, pos, rot);
        r.update_camera_buffer();
        let frame = scene.frame(&r, &game.world, frame_index as f32 * dt, &fox);
        let view_desc =
            FrameView::new(&camera, &frame.objects, &frame.lights).with_object_ids(&frame.ids);
        r.render_frame_view(&view_desc, &view).expect("frame");
        let _ = r.device.poll(wgpu::PollType::wait_indefinitely());
    }
    let pixels = read_texture_ktx2(&r.device, &r.queue, &target, false)
        .expect("read back")
        .levels
        .swap_remove(0);
    image::save_buffer(path, &pixels, W, H, image::ColorType::Rgba8).expect("save the PNG");
    println!(
        "saved {path}: {} gems, score {}, time left {:.1}",
        game.world.gems_taken(),
        game.world.score,
        game.world.time_left
    );
}

fn play() {
    let fox = load_fox();
    let saves = save::save_manager();
    let mut progress = save::Progress::load(&saves);

    let config = EngineConfig {
        window_title: "RedOx - Foxfire".to_string(),
        ..Default::default()
    };
    let (event_loop, window) = create_window(&config);
    let mut ctx = pollster::block_on(RenderContext::new(
        window.clone(),
        config.wgpu_present_mode(),
    ));
    ctx.set_render_scale(config.render_scale);
    tune(&mut ctx);
    let mut scene = Scene::new(&mut ctx, &fox).expect("scene");
    let mut game = Game::new(new_seed(), &fox);

    let mut ui_ctx = UiContext::new(&window, ctx.device(), ctx.surface_format());
    ui_ctx.show_debug = false;
    let mut loc = ui::localization();
    if let Some(language) = &progress.language {
        loc.set_language(language);
    }
    let mut screens = ScreenStack::default();
    screens.reset("menu", true);
    let mut focus = Focus::default();
    let mut input = InputState::new();
    let mut audio = AudioContext::new();
    sounds::register_all(&mut audio);
    let cues = CueLibrary::parse(sounds::CUES).expect("valid cue table");
    let mut selector = CueSelector::new();
    let mut pads = GamepadsState::new();
    let mut pad_backend = GilrsBackend::new();
    let mut pad_input = pad::PadInput::default();
    let (mut cursor, mut click) = ((0.0f32, 0.0f32), false);
    let mut activate = false;
    let mut nav: Option<Direction> = None;
    let mut escape = false;
    let mut last = Instant::now();
    let start = Instant::now();
    let mut orbit = 0.0f32;
    let mut end: Option<ui::EndData> = None;
    let mut hit_flash = 0.0f32;
    let mut was_exhausted = false;

    event_loop
        .run_closure(move |event, elwt| match event {
            winit::event::Event::NewEvents(_) => {
                input.begin_frame();
                pads.begin_frame();
                if let Some(backend) = pad_backend.as_mut() {
                    backend.poll(&mut pads);
                }
                pad_input = pad::read(&pads);
                escape |= pad_input.back;
                activate |= pad_input.confirm;
                nav = nav.or(pad_input.nav);
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
                                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                                    activate = true
                                }
                                KeyCode::ArrowDown | KeyCode::KeyS => nav = Some(Direction::Down),
                                KeyCode::ArrowUp | KeyCode::KeyW => nav = Some(Direction::Up),
                                KeyCode::Tab => nav = Some(Direction::Next),
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

                        // Escape: pause / resume / back.
                        let top = screens.top().map(|s| s.name.clone()).unwrap_or_default();
                        if std::mem::take(&mut escape) {
                            match top.as_str() {
                                "hud" => screens.push("pause", false),
                                "pause" => {
                                    screens.pop();
                                }
                                "end" => screens.reset("menu", true),
                                _ => {}
                            }
                        }
                        let top = screens.top().map(|s| s.name.clone()).unwrap_or_default();

                        // Simulation: the menu shows the fox standing in the glade and a slow orbit.
                        let playing = top == "hud";
                        let controls = if playing {
                            keyboard_controls(&input, &pads, &pad_input)
                        } else {
                            Controls::default()
                        };
                        if !matches!(top.as_str(), "pause" | "end") {
                            let (events, pose) = game.tick(dt, controls, orbit, &fox);
                            scene.pose_fox(&ctx, &fox, &pose);
                            if playing {
                                for _ in 0..events.gems {
                                    audio.play_cue(&cues, &mut selector, "gem", time);
                                }
                                if events.hit {
                                    hit_flash = 0.5;
                                    audio.play_cue(&cues, &mut selector, "hit", time);
                                }
                                if game.world.exhausted && !was_exhausted {
                                    audio.play_cue(&cues, &mut selector, "exhausted", time);
                                }
                                was_exhausted = game.world.exhausted;
                                if let Some(phase) = events.finished {
                                    let won = phase == Phase::Won;
                                    let record = progress.record(game.world.score);
                                    if record {
                                        progress.save(&saves);
                                    }
                                    audio.play_cue(
                                        &cues,
                                        &mut selector,
                                        if won { "win" } else { "lose" },
                                        time,
                                    );
                                    end = Some(ui::EndData {
                                        won,
                                        gems: game.world.gems_taken(),
                                        total: game.world.gems.len(),
                                        score: game.world.score,
                                        best: progress.best_score,
                                        new_record: record,
                                    });
                                    screens.push("end", false);
                                }
                            }
                        }
                        hit_flash = (hit_flash - dt).max(0.0);
                        orbit = if playing { 0.0 } else { orbit + 0.2 * dt };

                        let hero_bounds = scene.fox.swept_bounds().transform(
                            redox_math::Mat4::from_scale_rotation_translation(
                                Vec3::splat(fox::MODEL_SCALE),
                                Quat::from_rotation_y(game.world.yaw),
                                Vec3::new(game.world.pos.0, 0.0, game.world.pos.1),
                            ),
                        );
                        ctx.invalidate_shadow_region(&[hero_bounds]);

                        let camera = Camera::new(
                            FIELD_OF_VIEW,
                            ctx.config.width as f32 / ctx.config.height as f32,
                            0.1,
                            160.0,
                        );
                        let (pos, rot) = game.camera_pose();
                        ctx.camera_uniform.update(&camera, pos, rot);
                        ctx.update_camera_buffer();
                        let frame = scene.frame(&ctx, &game.world, time, &fox);
                        let fv = FrameView::new(&camera, &frame.objects, &frame.lights)
                            .with_object_ids(&frame.ids);
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
                                    label: Some("foxrun_ui"),
                                });
                        ui_ctx.begin_frame(&window);
                        let points = ui_ctx.egui_ctx.content_rect();
                        let scale = UiScale::new(points.width(), points.height());
                        let painter = ui_ctx.egui_ctx.layer_painter(egui::LayerId::new(
                            egui::Order::Foreground,
                            egui::Id::new("foxrun_ui"),
                        ));
                        let measure = egui_measure(&painter, scale);
                        let root = scale.root(0.03);
                        let world = &game.world;
                        let hud = ui::HudData {
                            gems: world.gems_taken(),
                            total: world.gems.len(),
                            score: world.score,
                            combo: world.combo,
                            time_left: world.time_left,
                            stamina: world.stamina,
                            exhausted: world.exhausted,
                            hit: hit_flash > 0.0,
                        };
                        let mut quit = false;
                        let visible: Vec<String> =
                            screens.visible().iter().map(|s| s.name.clone()).collect();
                        for name in visible {
                            let tree = match name.as_str() {
                                "menu" => ui::main_menu(progress.best_score),
                                "pause" => ui::pause_menu(),
                                "end" => match &end {
                                    Some(data) => ui::end_screen(data),
                                    None => ui::pause_menu(),
                                },
                                _ => ui::hud(&hud),
                            };
                            let laid = layout_localized(&tree, root, &measure, &loc);
                            let is_top = screens.top().is_some_and(|t| t.name == name);
                            if is_top && name != "hud" {
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
                                    if restart {
                                        game = Game::new(new_seed(), &fox);
                                        was_exhausted = false;
                                        end = None;
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

#[cfg(test)]
mod tests {
    use super::*;
    use world::{START_TIME, World};

    fn fox() -> FoxAssets {
        load_fox()
    }

    /// Plays a whole run with the autopilot at 60 fps.
    fn autopilot_run(seed: u64, limit: f32) -> World {
        let mut world = World::new(seed);
        let mut pilot = Autopilot::new();
        let dt = 1.0 / 60.0;
        let mut t = 0.0;
        while world.phase == Phase::Playing && t < limit {
            let controls = pilot.drive(&world, dt);
            world.step(dt, controls);
            t += dt;
        }
        world
    }

    #[test]
    fn a_run_can_be_won_with_a_simple_strategy() {
        let mut won = 0;
        for seed in [1u64, 2, 3, 4, 5] {
            let world = autopilot_run(seed, START_TIME + 60.0);
            if world.phase == Phase::Won {
                won += 1;
            }
            assert!(
                world.gems_taken() >= 20,
                "seed {seed}: only {} of {} gems",
                world.gems_taken(),
                world.gems.len()
            );
        }
        assert!(won >= 3, "the glade is winnable: {won}/5 runs won");
    }

    #[test]
    fn standing_still_loses_when_the_lantern_burns_out() {
        let mut world = World::new(9);
        for _ in 0..((START_TIME + 5.0) * 60.0) as u32 {
            world.step(1.0 / 60.0, Controls::default());
        }
        assert_eq!(world.phase, Phase::Lost);
        assert_eq!(world.gems_taken(), 0);
    }

    #[test]
    fn the_game_ticks_the_animation_with_the_speed() {
        let fox = fox();
        let mut game = Game::new(3, &fox);
        for _ in 0..60 {
            game.tick(1.0 / 60.0, Controls::default(), 0.0, &fox);
        }
        assert_eq!(game.graph.state_name(0), Some("idle"));
        let run = Controls {
            move_z: 1.0,
            sprint: true,
            ..Controls::default()
        };
        for _ in 0..60 {
            game.tick(1.0 / 60.0, run, 0.0, &fox);
        }
        assert_eq!(game.graph.state_name(0), Some("move"));
        assert!(game.world.speed > 5.0);
    }

    #[test]
    fn the_camera_follows_the_fox() {
        let fox = fox();
        let mut game = Game::new(3, &fox);
        let before = game.cam_pos;
        let run = Controls {
            move_z: 1.0,
            sprint: true,
            ..Controls::default()
        };
        for _ in 0..120 {
            game.tick(1.0 / 60.0, run, 0.0, &fox);
        }
        assert!(
            (game.cam_pos - before).length() > 5.0,
            "the camera moved along"
        );
        let (_, rotation) = game.camera_pose();
        assert!(rotation.is_normalized());
    }
}
