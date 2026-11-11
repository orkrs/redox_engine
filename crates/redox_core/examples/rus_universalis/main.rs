//! RedOx Universalis: 1380 CE (Куликовская битва и Русь в огне)
//!
//! A Grand Strategy Game prototype built on RedOx Engine:
//! - Full 3D tabletop strategy map with PBR lighting and VSM shadows
//! - Demographic POPs simulation (Peasants, Burghers, Clergy, Nobles)
//! - Natural goods & local production economy
//! - Active pause and time speed controls (1x to 5x)
//! - Army recruitment (EU5 Levies mechanic from peasant population)
//! - Historical narrative events with choices
//! - Interactive egui UI (Top bar, Province inspector, Army management)
//!
//! Run: `cargo run --example rus_universalis -p redox_core --release`
//! Controls:
//! - Pan: WASD / Arrow keys / Right-click drag
//! - Zoom: Mouse wheel
//! - Select: Left-click on province / army
//! - Move army: Right-click on neighbor province when army is selected
//! - Pause: Space bar | Speed: 1..5

#![allow(clippy::unwrap_used)]

mod camera;
mod data;
mod events;
mod map;
mod sim;
mod types;
mod ui;

use std::time::Instant;

use camera::StrategyCamera;
use data::build_historical_scenario;
use events::EventManager;
use map::MapAssets;
use sim::Simulation;
use types::*;
use ui::GameUi;

use redox_core::config::EngineConfig;
use redox_core::window::{RunClosure, create_window};
use redox_math::{Quat, Vec3};
use redox_render::context::RenderContext;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_ui::UiContext;
use winit::event::{ElementState, Event, MouseButton, MouseScrollDelta, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

fn create_lighting() -> LightScene {
    let mut sun = DirectionalLight::new(
        Vec3::new(-0.35, 1.0, 0.45).normalize(),
        Vec3::new(1.0, 0.95, 0.88),
        2.2,
    );
    sun.soft_shadows = false;
    sun.vsm = true;

    LightScene {
        directional: sun,
        ambient: Vec3::new(0.18, 0.20, 0.22),
        points: vec![],
        spots: vec![],
    }
}

fn main() {
    let _ = env_logger::try_init();
    log::info!("Запуск RedOx Universalis: 1380...");

    let mut config = EngineConfig::default();
    config.window_title = "RedOx Universalis: Русь 1380 (Project Caesar Lite)".to_string();
    config.window_width = 1600;
    config.window_height = 900;

    let (event_loop, window) = create_window(&config);

    let mut ctx = pollster::block_on(RenderContext::new(
        window.clone(),
        config.wgpu_present_mode(),
    ));

    ctx.set_lighting_mode(config.lighting_mode.into());
    ctx.set_aa_sharp(config.aa_sharp);
    ctx.set_render_scale(config.render_scale);

    let mut ui_ctx = UiContext::new(&window, ctx.device(), ctx.surface_format());
    let map_assets = MapAssets::new(&mut ctx);
    let mut camera = StrategyCamera::new();

    let (provinces, countries, armies) = build_historical_scenario();
    let mut sim = Simulation::new(provinces, countries, armies, CountryTag::Moscow);
    let mut events = EventManager::new();
    let mut game_ui = GameUi::new();

    let lights = create_lighting();

    // Input state
    let mut mouse_pos = (0.0_f32, 0.0_f32);
    let mut right_mouse_pressed = false;
    let mut last_drag_pos = (0.0_f32, 0.0_f32);
    let mut keys_held = [false; 4]; // W, A, S, D
    let mut last_frame_time = Instant::now();

    event_loop
        .run_closure(move |event, elwt| {
            match event {
                Event::WindowEvent { event, .. } => {
                    // Pass to egui first
                    let consumed_by_ui = ui_ctx.handle_window_event(&window, &event);

                    match event {
                        WindowEvent::CloseRequested => elwt.exit(),
                        WindowEvent::Resized(size) => {
                            if size.width > 0 && size.height > 0 {
                                ctx.resize(size.width, size.height);
                            }
                        }
                        WindowEvent::CursorMoved { position, .. } => {
                            mouse_pos = (position.x as f32, position.y as f32);
                            if right_mouse_pressed && !consumed_by_ui {
                                let dx = mouse_pos.0 - last_drag_pos.0;
                                let dy = mouse_pos.1 - last_drag_pos.1;
                                camera.pan(-dx * 0.015, -dy * 0.015);
                                last_drag_pos = mouse_pos;
                            }
                        }
                        WindowEvent::MouseWheel { delta, .. } => {
                            if !consumed_by_ui {
                                let scroll = match delta {
                                    MouseScrollDelta::LineDelta(_, y) => y,
                                    MouseScrollDelta::PixelDelta(pos) => (pos.y as f32) * 0.05,
                                };
                                camera.zoom(scroll);
                            }
                        }
                        WindowEvent::MouseInput { state, button, .. } => {
                            if button == MouseButton::Right {
                                match state {
                                    ElementState::Pressed => {
                                        right_mouse_pressed = true;
                                        last_drag_pos = mouse_pos;

                                        // Move army order on right click
                                        if !consumed_by_ui && game_ui.selected_army.is_some() {
                                            let aspect =
                                                ctx.config.width as f32 / ctx.config.height as f32;
                                            let cam = camera.to_camera(aspect);
                                            let cam_pos = camera.eye_position();
                                            let forward = (camera.target - cam_pos).normalize();
                                            let cam_rot =
                                                Quat::from_rotation_arc(Vec3::NEG_Z, forward);
                                            let inv_vp =
                                                cam.view_proj_matrix(cam_pos, cam_rot).inverse();

                                            if let Some(target_prov) = MapAssets::pick_province(
                                                mouse_pos.0,
                                                mouse_pos.1,
                                                ctx.config.width as f32,
                                                ctx.config.height as f32,
                                                inv_vp,
                                                &sim.provinces,
                                            ) {
                                                if let Some(army_id) = game_ui.selected_army {
                                                    let moved = sim.move_army(army_id, target_prov);
                                                    if moved {
                                                        let dest_name = sim
                                                            .provinces
                                                            .get(&target_prov)
                                                            .map_or("", |p| p.name_ru);
                                                        sim.battle_logs.push(format!(
                                                            "Войско выдвинулось в направлении: {}",
                                                            dest_name
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    ElementState::Released => {
                                        right_mouse_pressed = false;
                                    }
                                }
                            } else if button == MouseButton::Left && state == ElementState::Pressed
                            {
                                if !consumed_by_ui {
                                    let aspect = ctx.config.width as f32 / ctx.config.height as f32;
                                    let cam = camera.to_camera(aspect);
                                    let cam_pos = camera.eye_position();
                                    let forward = (camera.target - cam_pos).normalize();
                                    let cam_rot = Quat::from_rotation_arc(Vec3::NEG_Z, forward);
                                    let inv_vp = cam.view_proj_matrix(cam_pos, cam_rot).inverse();

                                    // Pick province
                                    if let Some(picked_prov) = MapAssets::pick_province(
                                        mouse_pos.0,
                                        mouse_pos.1,
                                        ctx.config.width as f32,
                                        ctx.config.height as f32,
                                        inv_vp,
                                        &sim.provinces,
                                    ) {
                                        game_ui.selected_province = Some(picked_prov);

                                        // Check if there is an army in this province to select
                                        let army_in_prov =
                                            sim.armies.iter().find(|a| a.location == picked_prov);
                                        if let Some(army) = army_in_prov {
                                            game_ui.selected_army = Some(army.id);
                                        } else {
                                            game_ui.selected_army = None;
                                        }
                                    }
                                }
                            }
                        }
                        WindowEvent::KeyboardInput { event, .. } => {
                            let pressed = event.state == ElementState::Pressed;
                            if let PhysicalKey::Code(code) = event.physical_key {
                                match code {
                                    KeyCode::KeyW | KeyCode::ArrowUp => keys_held[0] = pressed,
                                    KeyCode::KeyA | KeyCode::ArrowLeft => keys_held[1] = pressed,
                                    KeyCode::KeyS | KeyCode::ArrowDown => keys_held[2] = pressed,
                                    KeyCode::KeyD | KeyCode::ArrowRight => keys_held[3] = pressed,
                                    KeyCode::Space if pressed => {
                                        sim.paused = !sim.paused;
                                    }
                                    KeyCode::Digit1 if pressed => sim.speed = 1,
                                    KeyCode::Digit2 if pressed => sim.speed = 2,
                                    KeyCode::Digit3 if pressed => sim.speed = 3,
                                    KeyCode::Digit4 if pressed => sim.speed = 4,
                                    KeyCode::Digit5 if pressed => sim.speed = 5,
                                    KeyCode::KeyQ if pressed => camera.rotate_yaw(-0.15),
                                    KeyCode::KeyE if pressed => camera.rotate_yaw(0.15),
                                    _ => {}
                                }
                            }
                        }
                        WindowEvent::RedrawRequested => {
                            let now = Instant::now();
                            let dt = now.duration_since(last_frame_time).as_secs_f32().min(0.1);
                            last_frame_time = now;

                            // 1. Pan camera from keys
                            let mut pan_x = 0.0;
                            let mut pan_z = 0.0;
                            if keys_held[0] {
                                pan_z += 8.0 * dt;
                            }
                            if keys_held[2] {
                                pan_z -= 8.0 * dt;
                            }
                            if keys_held[1] {
                                pan_x -= 8.0 * dt;
                            }
                            if keys_held[3] {
                                pan_x += 8.0 * dt;
                            }
                            if pan_x != 0.0 || pan_z != 0.0 {
                                camera.pan(pan_x, pan_z);
                            }

                            camera.update(dt);

                            // 2. Advance simulation
                            sim.update(dt);
                            events.check_events(&mut sim);

                            // 3. Update Camera in GPU buffer
                            let aspect = ctx.config.width as f32 / ctx.config.height as f32;
                            let cam = camera.to_camera(aspect);
                            let cam_pos = camera.eye_position();
                            let forward = (camera.target - cam_pos).normalize();
                            let cam_rot = if forward.length_squared() > 1e-8 {
                                Quat::from_rotation_arc(Vec3::NEG_Z, forward)
                            } else {
                                Quat::IDENTITY
                            };

                            ctx.camera_uniform.update(&cam, cam_pos, cam_rot);
                            ctx.update_camera_buffer();

                            // 4. Hovered province query for highlight
                            let inv_vp = cam.view_proj_matrix(cam_pos, cam_rot).inverse();
                            let hovered_prov = MapAssets::pick_province(
                                mouse_pos.0,
                                mouse_pos.1,
                                ctx.config.width as f32,
                                ctx.config.height as f32,
                                inv_vp,
                                &sim.provinces,
                            );

                            // 5. Build render objects
                            let objects = map_assets.build_render_objects(
                                &sim,
                                game_ui.map_mode,
                                game_ui.selected_province,
                                hovered_prov,
                                game_ui.selected_army,
                            );

                            // 6. Render 3D Scene
                            let fv = FrameView::new(&cam, &objects, &lights);
                            if let Ok(output) = redox_render::acquire_surface_texture(ctx.surface())
                            {
                                let view = output
                                    .texture
                                    .create_view(&wgpu::TextureViewDescriptor::default());

                                if let Err(err) = ctx.render_frame_view(&fv, &view) {
                                    log::error!("Ошибка рендеринга кадра: {:?}", err);
                                }

                                // 7. Render UI overlay with egui
                                ui_ctx.record_frame(dt);
                                ui_ctx.begin_frame(&window);
                                game_ui.draw(&ui_ctx.egui_ctx, &mut sim, &mut events);

                                let mut encoder = ctx.device().create_command_encoder(
                                    &wgpu::CommandEncoderDescriptor {
                                        label: Some("universalis_ui_encoder"),
                                    },
                                );

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
                            }
                        }
                        _ => {}
                    }
                }
                Event::AboutToWait => {
                    window.request_redraw();
                }
                _ => {}
            }
        })
        .unwrap();
}
