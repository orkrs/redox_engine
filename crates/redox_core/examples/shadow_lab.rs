//! **Shadow Lab** — diagnostic scene for VSM / point-light shadows.
//!
//! Flat bright ground + tall casters so acne, peter-panning, clipmap seams,
//! and point-atlas artifacts are easy to spot.
//!
//! Run: `cargo run --example shadow_lab -p redox_core`
//!
//! WASD orbit · Q/E height · Arrows orbit sun · 1 shadows off · 2 VSM ·
//! 3 toggle point shadows · Space pause · R reset · ESC quit

// winit 0.30 deprecates the closure event loop; migrating to `ApplicationHandler` is tracked in the roadmap.
#![allow(deprecated)]

use std::f32::consts::TAU;
use std::time::Instant;

use redox_core::config::EngineConfig;
use redox_core::window::create_window;
use redox_ecs::World;
use redox_input::InputState;
use redox_math::{Quat, Vec3};
use redox_render::camera::Camera;
use redox_render::context::RenderContext;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene, PointLight};
use redox_render::material::Material;
use redox_render::mesh::primitive::{create_cube, create_sphere};
use redox_render::systems::{RenderObject, Transform};
use redox_ui::context::UiContext;
use winit::keyboard::KeyCode;

#[derive(Clone, Copy)]
struct Renderable {
    mesh_index: usize,
    material_index: usize,
}

struct LabState {
    yaw: f32,
    pitch: f32,
    dist: f32,
    sun_yaw: f32,
    sun_pitch: f32,
    use_vsm: bool,
    soft_shadows: bool,
    point_shadows: bool,
    point_count: u32,
    paused: bool,
    time: f32,
}

impl Default for LabState {
    fn default() -> Self {
        Self {
            yaw: 0.85,
            pitch: 0.55,
            dist: 18.0,
            sun_yaw: -0.7,
            sun_pitch: 0.85,
            use_vsm: false,
            soft_shadows: true,
            point_shadows: true,
            point_count: 4,
            paused: false,
            time: 0.0,
        }
    }
}

fn main() {
    env_logger::init();

    let config = EngineConfig {
        window_title: "RedOx — Shadow Lab".to_string(),
        window_width: 1400,
        window_height: 800,
        render_scale: 1.0,
        ..Default::default()
    };

    let (event_loop, window) = create_window(&config);
    let mut ctx = pollster::block_on(RenderContext::new(
        window.clone(),
        config.wgpu_present_mode(),
    ));
    ctx.set_exposure(1.05);
    ctx.set_render_scale(1.0);

    let mut ui = UiContext::new(&window, ctx.device(), ctx.surface_format());
    let mut world = World::new();
    world.insert_resource(InputState::new());

    build_scene(&mut world, &mut ctx);

    let mut lab = LabState::default();
    let mut last = Instant::now();

    event_loop
        .run(move |event, elwt| match event {
            winit::event::Event::NewEvents(_) => {
                if let Some(input) = world.get_resource_mut::<InputState>() {
                    input.begin_frame();
                }
            }
            winit::event::Event::WindowEvent { event, .. } => {
                if let Some(input) = world.get_resource_mut::<InputState>() {
                    input.process_window_event(&event);
                }
                if ui.handle_window_event(&window, &event) {
                    return;
                }
                match event {
                    winit::event::WindowEvent::CloseRequested => elwt.exit(),
                    winit::event::WindowEvent::Resized(size) => {
                        ctx.resize(size.width, size.height);
                    }
                    winit::event::WindowEvent::RedrawRequested => {
                        let dt = last.elapsed().as_secs_f32().min(0.05);
                        last = Instant::now();
                        if !lab.paused {
                            lab.time += dt;
                        }

                        handle_input(&world, &mut lab, dt, || elwt.exit());

                        let look = Vec3::new(0.0, 0.8, 0.0);
                        let (cam_pos, cam_rot) = orbit_camera(look, &lab);
                        let camera = Camera::new(
                            0.85,
                            ctx.config.width as f32 / ctx.config.height as f32,
                            0.1,
                            250.0,
                        );
                        ctx.camera_uniform.update(&camera, cam_pos, cam_rot);
                        ctx.update_camera_buffer();

                        let lights = build_lights(&lab);
                        ctx.set_vsm_enabled(lab.use_vsm);

                        let objects = extract_objects(&world, &ctx);
                        let fv = FrameView::new(&camera, &objects, &lights);

                        let output = match redox_render::acquire_surface_texture(ctx.surface()) {
                            Ok(t) => t,
                            Err(e) => {
                                log::error!("swapchain: {e}");
                                return;
                            }
                        };
                        let view = output.texture.create_view(&Default::default());
                        if let Err(e) = ctx.render_frame_view(&fv, &view) {
                            log::error!("render: {e}");
                            return;
                        }

                        ui.record_frame(dt);
                        ui.begin_frame(&window);
                        draw_hud(&ui.egui_ctx, &mut lab, &ctx, dt);
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
                    }
                    _ => {}
                }
            }
            winit::event::Event::AboutToWait => window.request_redraw(),
            _ => {}
        })
        .unwrap();
}

fn handle_input(world: &World, lab: &mut LabState, dt: f32, mut quit: impl FnMut()) {
    let Some(input) = world.get_resource::<InputState>() else {
        return;
    };
    if input.keyboard.is_pressed(KeyCode::Escape) {
        quit();
    }
    if input.keyboard.just_pressed(KeyCode::Space) {
        lab.paused = !lab.paused;
    }
    if input.keyboard.just_pressed(KeyCode::KeyR) {
        *lab = LabState::default();
    }
    if input.keyboard.just_pressed(KeyCode::Digit1) {
        lab.use_vsm = false;
        lab.soft_shadows = true;
    }
    if input.keyboard.just_pressed(KeyCode::Digit2) {
        lab.use_vsm = true;
        lab.soft_shadows = false;
    }
    if input.keyboard.just_pressed(KeyCode::Digit3) {
        lab.point_shadows = !lab.point_shadows;
    }
    if input.keyboard.just_pressed(KeyCode::Digit4) {
        lab.soft_shadows = !lab.soft_shadows;
        if lab.soft_shadows {
            lab.use_vsm = false;
        }
    }
    if input.keyboard.just_pressed(KeyCode::Digit5) {
        lab.point_count = match lab.point_count {
            0 => 1,
            1 => 2,
            2 => 4,
            4 => 6,
            _ => 0,
        };
    }

    let turn = 1.4 * dt;
    if input.keyboard.is_pressed(KeyCode::KeyA) {
        lab.yaw -= turn;
    }
    if input.keyboard.is_pressed(KeyCode::KeyD) {
        lab.yaw += turn;
    }
    if input.keyboard.is_pressed(KeyCode::KeyW) {
        lab.pitch = (lab.pitch + turn * 0.7).clamp(0.12, 1.35);
    }
    if input.keyboard.is_pressed(KeyCode::KeyS) {
        lab.pitch = (lab.pitch - turn * 0.7).clamp(0.12, 1.35);
    }
    if input.keyboard.is_pressed(KeyCode::KeyQ) {
        lab.dist = (lab.dist - 8.0 * dt).clamp(6.0, 40.0);
    }
    if input.keyboard.is_pressed(KeyCode::KeyE) {
        lab.dist = (lab.dist + 8.0 * dt).clamp(6.0, 40.0);
    }
    if input.keyboard.is_pressed(KeyCode::ArrowLeft) {
        lab.sun_yaw -= turn;
    }
    if input.keyboard.is_pressed(KeyCode::ArrowRight) {
        lab.sun_yaw += turn;
    }
    if input.keyboard.is_pressed(KeyCode::ArrowUp) {
        lab.sun_pitch = (lab.sun_pitch + turn * 0.6).clamp(0.15, 1.4);
    }
    if input.keyboard.is_pressed(KeyCode::ArrowDown) {
        lab.sun_pitch = (lab.sun_pitch - turn * 0.6).clamp(0.15, 1.4);
    }
}

fn orbit_camera(look: Vec3, lab: &LabState) -> (Vec3, Quat) {
    let offset = Vec3::new(
        lab.yaw.sin() * lab.pitch.cos() * lab.dist,
        lab.pitch.sin() * lab.dist,
        lab.yaw.cos() * lab.pitch.cos() * lab.dist,
    );
    let cam_pos = look + offset;
    let to = look - cam_pos;
    let rot = if to.length_squared() > 1e-8 {
        Quat::from_rotation_arc(Vec3::NEG_Z, to.normalize())
    } else {
        Quat::IDENTITY
    };
    (cam_pos, rot)
}

fn build_scene(world: &mut World, ctx: &mut RenderContext) {
    let cube = ctx.upload_mesh(&create_cube());
    let sphere = ctx.upload_mesh(&create_sphere(0.5, 32));

    // Bright ground — shadow contrast.
    let ground = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.82, 0.80, 0.76));
        m.roughness = 0.92;
        m
    });
    let dark = ctx.add_material(Material::solid(Vec3::new(0.12, 0.12, 0.14)));
    let red = ctx.add_material(Material::solid(Vec3::new(0.75, 0.18, 0.12)));
    let teal = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.15, 0.55, 0.55));
        m.metallic = 0.4;
        m.roughness = 0.35;
        m
    });
    let white = ctx.add_material(Material::solid(Vec3::new(0.92, 0.92, 0.94)));

    spawn(
        world,
        cube,
        ground,
        Vec3::new(0.0, -0.15, 0.0),
        Vec3::new(36.0, 0.3, 36.0),
    );

    // Checker markers for clipmap seams / swimming.
    for z in -3..=3 {
        for x in -3..=3 {
            if (x + z) % 2 != 0 {
                continue;
            }
            spawn(
                world,
                cube,
                dark,
                Vec3::new(x as f32 * 3.5, 0.02, z as f32 * 3.5),
                Vec3::new(1.6, 0.04, 1.6),
            );
        }
    }

    // Tall poles — long directional shadows.
    for (i, &(x, z)) in [(-6.0, -4.0), (-2.0, 5.0), (5.0, -3.0), (7.0, 6.0)]
        .iter()
        .enumerate()
    {
        let h = 3.0 + i as f32 * 0.8;
        spawn(
            world,
            cube,
            red,
            Vec3::new(x, h * 0.5, z),
            Vec3::new(0.45, h, 0.45),
        );
    }

    // Thin sticks — acne / peter-panning stress.
    for i in 0..6 {
        let a = i as f32 / 6.0 * TAU;
        spawn(
            world,
            cube,
            white,
            Vec3::new(a.cos() * 3.0, 1.4, a.sin() * 3.0),
            Vec3::new(0.12, 2.8, 0.12),
        );
    }

    // Stacked boxes — self-shadow contact.
    spawn(
        world,
        cube,
        teal,
        Vec3::new(0.0, 0.5, 0.0),
        Vec3::new(2.2, 1.0, 2.2),
    );
    spawn(
        world,
        cube,
        teal,
        Vec3::new(0.4, 1.35, -0.3),
        Vec3::new(1.4, 0.7, 1.4),
    );
    spawn(
        world,
        cube,
        teal,
        Vec3::new(-0.2, 2.0, 0.2),
        Vec3::new(0.9, 0.6, 0.9),
    );

    // Spheres — rounded contact shadows.
    spawn(
        world,
        sphere,
        red,
        Vec3::new(-4.5, 0.7, 1.5),
        Vec3::splat(1.4),
    );
    spawn(
        world,
        sphere,
        white,
        Vec3::new(3.5, 0.55, 4.0),
        Vec3::splat(1.1),
    );

    // “Stairs” ramp of boxes.
    for i in 0..5 {
        let t = i as f32;
        spawn(
            world,
            cube,
            dark,
            Vec3::new(9.0 + t * 0.9, 0.25 + t * 0.35, -6.0),
            Vec3::new(1.0, 0.5 + t * 0.7, 2.4),
        );
    }

    // Floating plank — shadow receiver vs detached caster.
    spawn(
        world,
        cube,
        white,
        Vec3::new(-8.0, 2.8, 2.0),
        Vec3::new(4.0, 0.18, 1.2),
    );
}

fn build_lights(lab: &LabState) -> LightScene {
    let sun_dir = Vec3::new(
        lab.sun_yaw.sin() * lab.sun_pitch.cos(),
        lab.sun_pitch.sin(),
        lab.sun_yaw.cos() * lab.sun_pitch.cos(),
    )
    .normalize();

    let mut sun = DirectionalLight::new(sun_dir, Vec3::new(1.0, 0.96, 0.88), 1.55);
    sun.soft_shadows = lab.soft_shadows && !lab.use_vsm;
    sun.vsm = lab.use_vsm;

    let mut points = Vec::new();
    let n = lab.point_count as usize;
    for i in 0..n {
        let a = i as f32 / n.max(1) as f32 * TAU + lab.time * 0.25;
        let r = 6.5;
        let mut l = PointLight::new(
            Vec3::new(a.cos() * r, 2.8 + (a * 2.0).sin() * 0.35, a.sin() * r),
            match i % 3 {
                0 => Vec3::new(1.0, 0.45, 0.25),
                1 => Vec3::new(0.35, 0.55, 1.0),
                _ => Vec3::new(0.4, 1.0, 0.5),
            },
            22.0,
            11.0,
        );
        l.cast_shadows = lab.point_shadows;
        points.push(l);
    }

    LightScene {
        directional: sun,
        // Low ambient — shadows must read clearly.
        ambient: Vec3::new(0.04, 0.045, 0.055),
        points,
        spots: vec![],
    }
}

fn extract_objects(world: &World, ctx: &RenderContext) -> Vec<RenderObject> {
    let mut out = Vec::new();
    for e in world.all_entities() {
        let (Some(t), Some(r)) = (
            world.get_component::<Transform>(e),
            world.get_component::<Renderable>(e),
        ) else {
            continue;
        };
        let bounds = ctx
            .get_mesh_bounds(r.mesh_index)
            .map(|b| b.transform(t.matrix()))
            .unwrap_or_else(|| redox_math::Aabb {
                min: Vec3::splat(-1e9),
                max: Vec3::splat(1e9),
            });
        out.push(RenderObject {
            model_matrix: t.matrix(),
            color: [1.0; 4],
            mesh_index: r.mesh_index,
            material_index: r.material_index,
            bounds,
        });
    }
    out
}

fn draw_hud(egui: &egui::Context, lab: &mut LabState, ctx: &RenderContext, dt: f32) {
    egui::Area::new(egui::Id::new("shadow_lab_hud"))
        .anchor(egui::Align2::LEFT_TOP, [12.0, 12.0])
        .show(egui, |ui| {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.heading("Shadow Lab");
                ui.monospace(format!("fps {:.0}", 1.0 / dt.max(1e-4)));
                ui.separator();

                let mode = if lab.use_vsm {
                    "VSM clipmap"
                } else {
                    "directional shadows off"
                };
                ui.label(format!("Directional: {mode}"));
                ui.label(format!(
                    "Point shadows: {} ({} lights)",
                    if lab.point_shadows { "ON" } else { "OFF" },
                    lab.point_count
                ));
                ui.label(format!(
                    "VSM dirty pages: {}",
                    ctx.vsm_dirty_pages_last_frame
                ));
                ui.label(format!(
                    "Sun pitch {:.2}  yaw {:.2}",
                    lab.sun_pitch, lab.sun_yaw
                ));
                if lab.paused {
                    ui.colored_label(egui::Color32::YELLOW, "PAUSED");
                }

                ui.separator();
                ui.checkbox(&mut lab.use_vsm, "VSM (2)");
                if lab.use_vsm {
                    lab.soft_shadows = false;
                }
                ui.checkbox(&mut lab.soft_shadows, "PCSS soft (4)");
                ui.checkbox(&mut lab.point_shadows, "Point cast shadows (3)");
                ui.add(egui::Slider::new(&mut lab.point_count, 0..=6).text("point lights (5)"));
                ui.add(egui::Slider::new(&mut lab.sun_pitch, 0.15..=1.4).text("sun elevation"));
                ui.add(egui::Slider::new(&mut lab.sun_yaw, -TAU..=TAU).text("sun azimuth"));

                ui.separator();
                ui.label("Look for: clipmap seams on checker,");
                ui.label("acne on thin sticks, contact under spheres,");
                ui.label("point-light multi-shadow overlap near center.");
                ui.separator();
                ui.label("WASD orbit · Q/E zoom · Arrows sun");
                ui.label("1 shadows off · 2 VSM · Space pause · R reset");
            });
        });
}

fn spawn(world: &mut World, mesh: usize, mat: usize, pos: Vec3, scale: Vec3) {
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
            material_index: mat,
        },
    );
}
