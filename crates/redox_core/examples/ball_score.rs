//! **Ball Score** — tiny collect-a-thon.
//!
//! Roll a ball on a bordered platform and pick up glowing orbs for points.
//!
//! Run: `cargo run --example ball_score -p redox_core`
//!
//! WASD move · Q/E camera · R restart · ESC quit

use std::time::Instant;

use redox_core::config::EngineConfig;
use redox_core::window::create_window;
use redox_ecs::{Entity, World};
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

const PLATFORM: f32 = 28.0;
const BORDER_H: f32 = 1.2;
const BALL_R: f32 = 0.55;
const ORB_COUNT: usize = 10;
const MOVE_SPEED: f32 = 9.0;

#[derive(Clone, Copy)]
struct Renderable {
    mesh_index: usize,
    material_index: usize,
}

struct Ball;
struct Orb {
    bob: f32,
}

struct Assets {
    sphere: usize,
    ball_mat: usize,
    orb_mat: usize,
}

struct Session {
    ball: Entity,
    orbs: Vec<Entity>,
    score: u32,
    camera_yaw: f32,
    time: f32,
}

fn main() {
    env_logger::init();

    let config = EngineConfig {
        window_title: "RedOx — Ball Score".to_string(),
        window_width: 1280,
        window_height: 720,
        render_scale: 1.0,
        ..Default::default()
    };

    let (event_loop, window) = create_window(&config);
    let mut ctx = pollster::block_on(RenderContext::new(
        window.clone(),
        config.wgpu_present_mode(),
    ));
    ctx.set_exposure(1.1);

    let mut ui = UiContext::new(&window, ctx.device(), ctx.surface_format());
    let mut world = World::new();
    world.insert_resource(InputState::new());

    let assets = build_level(&mut world, &mut ctx);
    let mut session = spawn_session(&mut world, &assets);
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

                        if world
                            .get_resource::<InputState>()
                            .is_some_and(|i| i.keyboard.is_pressed(KeyCode::Escape))
                        {
                            elwt.exit();
                        }

                        let input = world.get_resource::<InputState>().cloned();
                        tick(&mut world, &assets, &mut session, input.as_ref(), dt);

                        let (cam_pos, cam_rot, camera) = update_camera(&world, &session, &ctx);
                        ctx.camera_uniform.update(&camera, cam_pos, cam_rot);
                        ctx.update_camera_buffer();

                        let lights = build_lights(&world, &session);
                        let objects = extract_objects(&world, &ctx);
                        let fv = FrameView::new(&camera, &objects, &lights);

                        let output = ctx.surface().get_current_texture().unwrap();
                        let view = output.texture.create_view(&Default::default());
                        ctx.render_frame_view(&fv, &view).unwrap();

                        ui.record_frame(dt);
                        ui.begin_frame(&window);
                        draw_hud(&ui.egui_ctx, &session, dt);
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

fn build_level(world: &mut World, ctx: &mut RenderContext) -> Assets {
    let cube = ctx.upload_mesh(&create_cube());
    let sphere = ctx.upload_mesh(&create_sphere(BALL_R, 28));

    let floor_mat = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.18, 0.22, 0.28));
        m.roughness = 0.9;
        m
    });
    let wall_mat = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.12, 0.35, 0.55));
        m.metallic = 0.35;
        m.roughness = 0.4;
        m
    });
    let ball_mat = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.95, 0.35, 0.15));
        m.metallic = 0.15;
        m.roughness = 0.25;
        m.emissive = Vec3::new(0.15, 0.04, 0.01);
        m
    });
    let orb_mat = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.3, 0.95, 1.0));
        m.emissive = Vec3::new(0.6, 1.6, 2.2);
        m.metallic = 0.8;
        m.roughness = 0.12;
        m
    });

    // Floor
    spawn_mesh(
        world,
        cube,
        floor_mat,
        Vec3::new(0.0, -0.25, 0.0),
        Vec3::new(PLATFORM, 0.5, PLATFORM),
    );

    // Borders
    let half = PLATFORM * 0.5;
    let thick = 0.55;
    for &(x, z, sx, sz) in &[
        (0.0, -half, PLATFORM + thick, thick),
        (0.0, half, PLATFORM + thick, thick),
        (-half, 0.0, thick, PLATFORM),
        (half, 0.0, thick, PLATFORM),
    ] {
        spawn_mesh(
            world,
            cube,
            wall_mat,
            Vec3::new(x, BORDER_H * 0.5, z),
            Vec3::new(sx, BORDER_H, sz),
        );
    }

    Assets {
        sphere,
        ball_mat,
        orb_mat,
    }
}

fn spawn_session(world: &mut World, assets: &Assets) -> Session {
    let ball = world.spawn();
    world.add_component(
        ball,
        Transform {
            translation: Vec3::new(0.0, BALL_R, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        },
    );
    world.add_component(
        ball,
        Renderable {
            mesh_index: assets.sphere,
            material_index: assets.ball_mat,
        },
    );
    world.add_component(ball, Ball);

    let spots = [
        Vec3::new(8.0, 0.0, 6.0),
        Vec3::new(-7.0, 0.0, 8.0),
        Vec3::new(5.0, 0.0, -9.0),
        Vec3::new(-9.0, 0.0, -5.0),
        Vec3::new(0.0, 0.0, 10.0),
        Vec3::new(10.0, 0.0, -2.0),
        Vec3::new(-4.0, 0.0, -10.0),
        Vec3::new(6.0, 0.0, 10.0),
        Vec3::new(-10.0, 0.0, 2.0),
        Vec3::new(2.0, 0.0, -6.0),
    ];

    let mut orbs = Vec::new();
    for (i, pos) in spots.into_iter().take(ORB_COUNT).enumerate() {
        let e = world.spawn();
        world.add_component(
            e,
            Transform {
                translation: Vec3::new(pos.x, 0.85, pos.z),
                rotation: Quat::IDENTITY,
                scale: Vec3::splat(0.55),
            },
        );
        world.add_component(
            e,
            Renderable {
                mesh_index: assets.sphere,
                material_index: assets.orb_mat,
            },
        );
        world.add_component(e, Orb { bob: i as f32 * 0.7 });
        orbs.push(e);
    }

    Session {
        ball,
        orbs,
        score: 0,
        camera_yaw: 0.35,
        time: 0.0,
    }
}

fn reset(world: &mut World, assets: &Assets, session: &mut Session) {
    world.despawn(session.ball);
    for e in session.orbs.drain(..) {
        world.despawn(e);
    }
    *session = spawn_session(world, assets);
}

fn tick(
    world: &mut World,
    assets: &Assets,
    session: &mut Session,
    input: Option<&InputState>,
    dt: f32,
) {
    if input.is_some_and(|i| i.keyboard.just_pressed(KeyCode::KeyR)) {
        reset(world, assets, session);
        return;
    }

    session.time += dt;

    if let Some(input) = input {
        if input.keyboard.is_pressed(KeyCode::KeyQ) {
            session.camera_yaw -= dt * 1.8;
        }
        if input.keyboard.is_pressed(KeyCode::KeyE) {
            session.camera_yaw += dt * 1.8;
        }
    }

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

    if let Some(t) = world.get_component_mut::<Transform>(session.ball) {
        if move_dir.length_squared() > 0.0 {
            let dir = move_dir.normalize();
            t.translation += dir * MOVE_SPEED * dt;

            // Visual roll
            let axis = Vec3::new(-dir.z, 0.0, dir.x);
            let angle = (MOVE_SPEED * dt) / BALL_R;
            t.rotation = Quat::from_axis_angle(axis, angle) * t.rotation;
        }

        let limit = PLATFORM * 0.5 - BALL_R - 0.35;
        t.translation.x = t.translation.x.clamp(-limit, limit);
        t.translation.z = t.translation.z.clamp(-limit, limit);
        t.translation.y = BALL_R;
    }

    let ball_pos = world
        .get_component::<Transform>(session.ball)
        .map(|t| t.translation)
        .unwrap_or(Vec3::ZERO);

    for &orb in &session.orbs {
        if let Some(o) = world.get_component_mut::<Orb>(orb) {
            o.bob += dt * 2.4;
            let bob = o.bob;
            if let Some(t) = world.get_component_mut::<Transform>(orb) {
                t.translation.y = 0.85 + bob.sin() * 0.22;
                t.rotation = Quat::from_rotation_y(bob);
            }
        }
    }

    let mut picked = Vec::new();
    session.orbs.retain(|&e| {
        let pos = world
            .get_component::<Transform>(e)
            .map(|t| t.translation)
            .unwrap_or(Vec3::ZERO);
        if (ball_pos - pos).length() < 1.15 {
            picked.push(e);
            false
        } else {
            true
        }
    });
    for e in picked {
        world.despawn(e);
        session.score += 10;
    }
}

fn update_camera(world: &World, session: &Session, ctx: &RenderContext) -> (Vec3, Quat, Camera) {
    let target = world
        .get_component::<Transform>(session.ball)
        .map(|t| t.translation + Vec3::new(0.0, 0.35, 0.0))
        .unwrap_or(Vec3::Y);
    // Closer chase cam, steeper pitch so the platform fills the view.
    let dist = 8.5;
    let pitch = 0.95_f32;
    let yaw = session.camera_yaw;
    let offset = Vec3::new(
        yaw.sin() * pitch.cos() * dist,
        pitch.sin() * dist,
        yaw.cos() * pitch.cos() * dist,
    );
    let cam_pos = target + offset;
    let to_target = target - cam_pos;
    let rotation = if to_target.length_squared() > 1.0e-6 {
        Quat::from_rotation_arc(Vec3::NEG_Z, to_target.normalize())
    } else {
        Quat::IDENTITY
    };
    let camera = Camera::new(
        0.9,
        ctx.config.width as f32 / ctx.config.height as f32,
        0.1,
        200.0,
    );
    (cam_pos, rotation, camera)
}

fn build_lights(world: &World, session: &Session) -> LightScene {
    let mut sun = DirectionalLight::new(
        Vec3::new(-0.35, 1.0, 0.4),
        Vec3::new(1.0, 0.95, 0.88),
        1.15,
    );
    sun.soft_shadows = true;

    let mut points = Vec::new();
    for &orb in &session.orbs {
        if let Some(t) = world.get_component::<Transform>(orb) {
            let mut l = PointLight::new(
                t.translation + Vec3::new(0.0, 0.3, 0.0),
                Vec3::new(0.35, 0.85, 1.0),
                8.0,
                5.5,
            );
            l.cast_shadows = false;
            points.push(l);
        }
    }
    // Soft fill so the empty platform is not pitch black after collecting all.
    if points.is_empty() {
        let mut l = PointLight::new(
            Vec3::new(0.0, 4.0, 0.0),
            Vec3::new(0.5, 0.7, 1.0),
            6.0,
            18.0,
        );
        l.cast_shadows = false;
        points.push(l);
    }

    LightScene {
        directional: sun,
        ambient: Vec3::new(0.07, 0.08, 0.1),
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
    }
    out
}

fn draw_hud(egui: &egui::Context, session: &Session, dt: f32) {
    egui::Area::new(egui::Id::new("hud"))
        .anchor(egui::Align2::LEFT_TOP, [14.0, 14.0])
        .show(egui, |ui| {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.heading("Ball Score");
                ui.label(format!("Score: {}", session.score));
                ui.label(format!("Orbs left: {}", session.orbs.len()));
                ui.label(format!("Time: {:.1}s", session.time));
                ui.label(format!("FPS: {:.0}", 1.0 / dt.max(1e-4)));
                if session.orbs.is_empty() {
                    ui.colored_label(
                        egui::Color32::from_rgb(90, 220, 140),
                        "All orbs collected! Press R to replay",
                    );
                }
                ui.separator();
                ui.label("WASD · Q/E · R restart · ESC quit");
            });
        });
}

fn spawn_mesh(
    world: &mut World,
    mesh: usize,
    material: usize,
    pos: Vec3,
    scale: Vec3,
) -> Entity {
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
