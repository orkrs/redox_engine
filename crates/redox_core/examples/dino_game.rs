//! Dino Game 3D — Side-scroller Ultra Edition
//! Fixes: Input cycle, Vulkan Sync, Side-view camera, High Jump.

use std::sync::Arc;
use std::time::Instant;
use winit::{
    event::{Event, WindowEvent},
    event_loop::EventLoop,
    window::WindowBuilder,
    keyboard::KeyCode,
};

use redox_ecs::world::World;
use redox_math::{Transform, Vec3, Quat, Mat4};
use redox_render::{
    camera::Camera,
    light::{DirectionalLight, LightUniform, PointLight},
    mesh::primitive::{create_cube, create_sphere},
    systems::{MaterialHandle, MeshHandle, sync_assets_to_render, extract_render_objects},
    asset_types::{MeshData, MaterialData},
    RenderContext
};
use redox_input::state::InputState;
use redox_ui::context::UiContext;
use redox_asset::{AssetManager, Handle};

// -----------------------------------------------------------------------------
// Constants
// -----------------------------------------------------------------------------
const GROUND_Y: f32 = 0.0;
const PLAYER_SIZE: Vec3 = Vec3::new(0.8, 0.8, 0.8);
const OBSTACLE_SIZE: Vec3 = Vec3::new(0.6, 1.4, 0.6); // Чуть выше
const SPAWN_Z: f32 = 20.0;
const DESPAWN_Z: f32 = -15.0;

// Прыжок стал выше
const JUMP_VELOCITY: f32 = 13.0;
const GRAVITY: f32 = 32.0;

// Вид сбоку: Камера смещена по X
const CAMERA_POS: Vec3 = Vec3::new(16.0, 5.0, 0.0);
const CAMERA_TARGET: Vec3 = Vec3::new(0.0, 2.0, 0.0);

const SCORE_INTERVAL: f32 = 0.15;

// -----------------------------------------------------------------------------
// Components & Resources
// -----------------------------------------------------------------------------
struct Player;
struct Obstacle;

#[derive(PartialEq, Clone, Copy)]
enum GameState { Running, GameOver }

struct GameResources {
    state: GameState,
    score: u32,
    speed: f32,
    next_spawn_time: f32,
    spawn_interval: f32,
}

#[derive(Default, Clone, Copy)]
struct JumpState {
    velocity_y: f32,
    is_grounded: bool,
}

#[derive(Clone)]
struct GameAssets {
    cube_mesh: Handle<MeshData>,
    sphere_mesh: Handle<MeshData>,
    player_mat: Handle<MaterialData>,
    obstacle_mat: Handle<MaterialData>,
    ground_mat: Handle<MaterialData>,
}

// -----------------------------------------------------------------------------
// Main
// -----------------------------------------------------------------------------
fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Info).init();
    pollster::block_on(run());
}

async fn run() {
    let event_loop = EventLoop::new().unwrap();
    let window = Arc::new(WindowBuilder::new()
        .with_title("RedOx Engine — Dino Ultra 2.5D")
        .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0))
        .build(&event_loop).unwrap());

    let mut render_ctx = RenderContext::new(window.clone()).await;
    let mut ui_ctx = UiContext::new(&window, render_ctx.device(), render_ctx.surface_format());
    let mut asset_manager = AssetManager::default();
    let mut world = World::new();
    let mut input_state = InputState::new();

    // Assets
    let cube = asset_manager.insert(create_cube());
    let sphere = asset_manager.insert(create_sphere(0.5, 32));
    let p_mat = asset_manager.insert(MaterialData::solid(Vec3::new(1.0, 0.3, 0.1)).metallic(0.8).roughness(0.1));
    let o_mat = asset_manager.insert(MaterialData::solid(Vec3::new(0.2, 0.9, 0.4)).roughness(0.7));
    let g_mat = asset_manager.insert(MaterialData::solid(Vec3::new(0.1, 0.1, 0.12)).roughness(0.2));

    let assets = GameAssets { cube_mesh: cube, sphere_mesh: sphere, player_mat: p_mat, obstacle_mat: o_mat, ground_mat: g_mat };

    world.insert_resource(assets.clone());
    world.insert_resource(GameResources { state: GameState::Running, score: 0, speed: 10.0, next_spawn_time: 1.0, spawn_interval: 0.9 });
    world.insert_resource(JumpState::default());

    // Entities
    let player = world.spawn();
    world.add_component(player, Transform { translation: Vec3::new(0.0, GROUND_Y, 0.0), rotation: Quat::IDENTITY, scale: PLAYER_SIZE });
    world.add_component(player, MeshHandle(assets.sphere_mesh.clone()));
    world.add_component(player, MaterialHandle(assets.player_mat.clone()));
    world.add_component(player, Player);

    let ground = world.spawn();
    world.add_component(ground, Transform { translation: Vec3::new(0.0, -0.5, 0.0), rotation: Quat::IDENTITY, scale: Vec3::new(2.0, 0.1, 200.0) });
    world.add_component(ground, MeshHandle(assets.cube_mesh.clone()));
    world.add_component(ground, MaterialHandle(assets.ground_mat.clone()));

    let sun = world.spawn();
    world.add_component(sun, DirectionalLight {
        direction: Vec3::new(-1.0, -2.0, -0.5).normalize(),
        color: Vec3::new(1.0, 0.9, 0.8),
        intensity: 2.5,
        cast_vsm_shadows: false,
        source_angle: 0.1
    });

    let camera_entity = world.spawn();
    let view_mat = Mat4::look_at_rh(CAMERA_POS, CAMERA_TARGET, Vec3::Y);
    world.add_component(camera_entity, Camera::new(45.0_f32.to_radians(), 1280.0/720.0, 0.1, 200.0));
    world.add_component(camera_entity, Transform { translation: CAMERA_POS, rotation: Quat::from_mat4(&view_mat.inverse()), scale: Vec3::ONE });

    let mut last_time = Instant::now();

    #[allow(deprecated)]
    event_loop.run(move |event, control_flow| {
        if let Event::WindowEvent { event: ref we, .. } = event {
            if ui_ctx.handle_window_event(&window, we) { return; }
            input_state.process_window_event(we);
        }

        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => control_flow.exit(),
            Event::WindowEvent { event: WindowEvent::Resized(s), .. } => {
                render_ctx.resize(s.width, s.height);
                if let Some(c) = world.get_component_mut::<Camera>(camera_entity) { c.aspect_ratio = s.width as f32 / s.height as f32; }
            }
            Event::AboutToWait => {
                window.request_redraw();
            }
            Event::WindowEvent { event: WindowEvent::RedrawRequested, .. } => {
                let dt = last_time.elapsed().as_secs_f32().min(0.033);
                last_time = Instant::now();

                // --- ЛОГИКА ИГРЫ ---
                let mut game = world.remove_resource::<GameResources>().unwrap();

                // Обработка РЕСТАРТА
                if input_state.keyboard.just_pressed(KeyCode::KeyR) {
                    game = GameResources { state: GameState::Running, score: 0, speed: 10.0, next_spawn_time: 1.0, spawn_interval: 0.9 };
                    if let Some(tf) = world.get_component_mut::<Transform>(player) { tf.translation.y = GROUND_Y; }
                    if let Some(js) = world.get_resource_mut::<JumpState>() { *js = JumpState::default(); }
                    let obs: Vec<_> = world.all_entities().filter(|e| world.get_component::<Obstacle>(*e).is_some()).collect();
                    for e in obs { world.despawn(e); }
                }

                if game.state == GameState::Running {
                    // ПРЫЖОК
                    let mut jump = world.remove_resource::<JumpState>().unwrap();
                    if input_state.keyboard.just_pressed(KeyCode::Space) && jump.is_grounded {
                        jump.velocity_y = JUMP_VELOCITY;
                        jump.is_grounded = false;
                    }

                    jump.velocity_y -= GRAVITY * dt;
                    if let Some(tf) = world.get_component_mut::<Transform>(player) {
                        tf.translation.y += jump.velocity_y * dt;
                        if tf.translation.y <= GROUND_Y {
                            tf.translation.y = GROUND_Y;
                            jump.velocity_y = 0.0;
                            jump.is_grounded = true;
                        }
                    }
                    world.insert_resource(jump);

                    // СПАВН
                    game.next_spawn_time -= dt;
                    if game.next_spawn_time <= 0.0 {
                        let o = world.spawn();
                        world.add_component(o, Transform { translation: Vec3::new(0.0, GROUND_Y, SPAWN_Z), rotation: Quat::IDENTITY, scale: OBSTACLE_SIZE });
                        world.add_component(o, MeshHandle(assets.cube_mesh.clone()));
                        world.add_component(o, MaterialHandle(assets.obstacle_mat.clone()));
                        world.add_component(o, Obstacle);
                        game.next_spawn_time = game.spawn_interval;
                        game.score += 1;
                        game.speed += 0.1;
                    }

                    // ДВИЖЕНИЕ И КОЛЛИЗИИ
                    let p_pos = world.get_component::<Transform>(player).unwrap().translation;
                    let obs_entities: Vec<_> = world.all_entities().filter(|e| world.get_component::<Obstacle>(*e).is_some()).collect();
                    let mut to_del = Vec::new();
                    for e in obs_entities {
                        if let Some(tf) = world.get_component_mut::<Transform>(e) {
                            tf.translation.z -= game.speed * dt;
                            if tf.translation.z < DESPAWN_Z { to_del.push(e); }
                            // Тщательная коллизия (AABB упрощенный)
                            let dist = tf.translation.distance(p_pos);
                            if dist < 1.0 { game.state = GameState::GameOver; }
                        }
                    }
                    for e in to_del { world.despawn(e); }
                }
                world.insert_resource(game);

                // --- ПОДГОТОВКА РЕНДЕРА ---
                asset_manager.update(&mut world);
                let current_assets = world.get_resource::<GameAssets>().unwrap().clone();
                sync_assets_to_render(&mut render_ctx, &asset_manager,
                                      &[current_assets.cube_mesh, current_assets.sphere_mesh], &[],
                                      &[current_assets.player_mat, current_assets.obstacle_mat, current_assets.ground_mat]);

                if let (Some(cam), Some(tf)) = (world.get_component::<Camera>(camera_entity), world.get_component::<Transform>(camera_entity)) {
                    render_ctx.camera_uniform.update(cam, tf.translation, tf.rotation);
                    render_ctx.update_camera_buffer();
                }

                // Свет и Тени
                let shadow_matrix = {
                    let s = 30.0;
                    let proj = redox_math::orthographic(-s, s, -s, s, -50.0, 50.0);
                    let view = redox_math::look_at(Vec3::new(10.0, 20.0, 0.0), Vec3::ZERO, Vec3::Y);
                    proj * view
                };

                if let Some(light) = world.get_component::<DirectionalLight>(sun) {
                    let mut lu = LightUniform::new(light, Vec3::new(0.1, 0.1, 0.15));
                    lu.shadow_view_proj = shadow_matrix.to_cols_array_2d();
                    if let Some(ptf) = world.get_component::<Transform>(player) {
                        lu.add_point_light(&PointLight::new(ptf.translation + Vec3::new(0.0, 1.5, 0.0), Vec3::new(1.0, 0.6, 0.2), 8.0, 15.0));
                    }
                    render_ctx.update_light_buffer(&lu);
                }

                let render_objects = extract_render_objects(&world, &render_ctx);
                render_ctx.update_model_buffer(&render_objects);

                // --- РЕНДЕР ПАССЫ (Vulkan-safe sequence) ---
                let output = render_ctx.surface().get_current_texture().unwrap();
                let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
                let mut encoder = render_ctx.device().create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

                // 1. Shadows
                {
                    let shadow_matrix_buf = redox_render::resource::buffer::create_uniform_buffer(render_ctx.device(), "sh", bytemuck::bytes_of(&shadow_matrix.to_cols_array_2d()));
                    let shadow_bg = render_ctx.device().create_bind_group(&wgpu::BindGroupDescriptor {
                        label: None, layout: &render_ctx.shadow_pass.bind_group_layout,
                        entries: &[wgpu::BindGroupEntry { binding: 0, resource: shadow_matrix_buf.as_entire_binding() }],
                    });
                    let mut s_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Shadows"), color_attachments: &[],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &render_ctx.shadow_view, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                            stencil_ops: None,
                        }), ..Default::default()
                    });
                    s_pass.set_pipeline(&render_ctx.shadow_pass.pipeline);
                    s_pass.set_bind_group(0, &shadow_bg, &[]);
                    s_pass.set_bind_group(1, &render_ctx.shadow_model_bind_group, &[]);
                    for (i, obj) in render_objects.iter().enumerate() {
                        if let Some(m) = render_ctx.meshes.get(obj.mesh_index) {
                            s_pass.set_vertex_buffer(0, m.vertex_buffer.slice(..));
                            s_pass.set_index_buffer(m.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                            s_pass.draw_indexed(0..m.index_count, 0, (i as u32)..(i as u32 + 1));
                        }
                    }
                }

                // 2. Scene HDR
                {
                    let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("HDR Scene"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &render_ctx.hdr_view, resolve_target: None,
                            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.01, g: 0.01, b: 0.03, a: 1.0 }), store: wgpu::StoreOp::Store },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &render_ctx.depth_view, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                            stencil_ops: None,
                        }), ..Default::default()
                    });
                    render_ctx.record_draw(&mut rpass, &render_objects);
                }

                // 3. Tone Mapping
                {
                    let mut f_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Final"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view, resolve_target: None,
                            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                        })], ..Default::default()
                    });
                    f_pass.set_pipeline(&render_ctx.tone_mapping_pipeline);
                    f_pass.set_bind_group(0, &render_ctx.tone_mapping_bind_group, &[]);
                    f_pass.draw(0..3, 0..1);
                }

                // 4. UI
                ui_ctx.record_frame(dt);
                ui_ctx.begin_frame(&window);
                if let Some(res) = world.get_resource::<GameResources>() {
                    egui::Area::new(egui::Id::new("HUD")).anchor(egui::Align2::CENTER_TOP, [0.0, 60.0]).show(&ui_ctx.egui_ctx, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.heading(egui::RichText::new(format!("SCORE: {}", res.score)).size(60.0).strong().color(egui::Color32::WHITE));
                            if res.state == GameState::GameOver {
                                ui.heading(egui::RichText::new("GAME OVER").size(100.0).color(egui::Color32::RED).strong());
                                ui.label(egui::RichText::new("Press 'R' to Restart").size(30.0).color(egui::Color32::YELLOW));
                            }
                        });
                    });
                }
                ui_ctx.draw_debug(&mut world);
                ui_ctx.end_frame_and_render(render_ctx.device(), render_ctx.queue(), &mut encoder, &window, &view, egui_wgpu::ScreenDescriptor { size_in_pixels: [render_ctx.config().width, render_ctx.config().height], pixels_per_point: window.scale_factor() as f32 });

                // ONE submit, ONE present
                render_ctx.queue().submit(std::iter::once(encoder.finish()));
                output.present();

                // ОЧИСТКА ВВОДА В КОНЦЕ КАДРА
                input_state.begin_frame();
            }
            _ => {}
        }
    });
}