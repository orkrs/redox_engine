use std::sync::Arc;
use std::time::Instant;
use winit::{
    event::{Event, WindowEvent},
    event_loop::EventLoop,
    window::WindowBuilder,
};

// omit time since we'll compute it locally
use redox_ecs::world::World;
use redox_math::{Mat4, Quat, Transform, Vec3};

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder, RigidBodyHandle, Vector};
use redox_physics::context::PhysicsContext;
use redox_physics::sync::{step_physics, sync_from_physics};

use redox_render::camera::Camera;
use redox_render::context::RenderContext;
use redox_render::light::{DirectionalLight, LightUniform, PointLight};
use redox_render::mesh::primitive::{create_cube, create_sphere};
use redox_render::systems::{MaterialHandle, MeshHandle, extract_render_objects, sync_assets_to_render};
use redox_render::{asset_types::MaterialData, asset_types::MeshData};
use redox_asset::{AssetManager, Handle};

use redox_audio::components::{AudioEmitter, AudioListener};
use redox_audio::context::AudioContext;
use redox_input::{
    action::{ActionBinding, ActionMap},
    state::InputState,
};

use egui;
use egui::Color32;
use rand::Rng;
use redox_ui::context::UiContext;

// --- ECS Components & Resources ---

/// Marker for the player entity
#[derive(Debug, Clone)]
struct Player(RigidBodyHandle);

/// Marker for a collectible item (cube)
#[derive(Debug, Clone)]
struct Collectible;

/// Resource to hold the current score
struct GameScore(u32);

/// All mesh and material handles used in the scene (for sync_assets_to_render each frame).
struct GameAssets {
    mesh_handles: Vec<Handle<MeshData>>,
    material_handles: Vec<Handle<MaterialData>>,
}

// --- Game Logic ---

fn main() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();
    pollster::block_on(run());
}

async fn run() {
    log::info!("🚀 Starting Simple Game...");

    let event_loop = EventLoop::new().unwrap();
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("RedOx Engine - Simple Game")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0))
            .build(&event_loop)
            .unwrap(),
    );

    // Initialize subsystems
    let mut render_ctx = RenderContext::new(window.clone()).await;
    let mut ui_ctx = UiContext::new(&window, render_ctx.device(), render_ctx.surface_format());
    let audio_ctx = AudioContext::new();

    let mut world = World::new();
    // Add Physics Resource
    world.insert_resource(PhysicsContext::new(Vec3::new(0.0, -20.0, 0.0)));

    // Setup Game State Resource
    world.insert_resource(GameScore(0));

    // Add Audio Context
    world.insert_resource(audio_ctx);

    // Setup Input
    let mut input_state = InputState::new();
    let mut actions = ActionMap::new();
    actions.add(
        "MoveForward",
        ActionBinding::Key(winit::keyboard::KeyCode::KeyW),
    );
    actions.add(
        "MoveBackward",
        ActionBinding::Key(winit::keyboard::KeyCode::KeyS),
    );
    actions.add(
        "MoveLeft",
        ActionBinding::Key(winit::keyboard::KeyCode::KeyA),
    );
    actions.add(
        "MoveRight",
        ActionBinding::Key(winit::keyboard::KeyCode::KeyD),
    );
    actions.add("Reset", ActionBinding::Key(winit::keyboard::KeyCode::KeyR));

    // --- Assets (via AssetManager, synced to RenderContext each frame) ---
    let mut asset_manager = AssetManager::new(".");
    let mesh_sphere_handle = asset_manager.insert(create_sphere(0.5, 32));
    let mesh_cube_handle = asset_manager.insert(create_cube());
    let mesh_sphere = MeshHandle(mesh_sphere_handle);
    let mesh_cube = MeshHandle(mesh_cube_handle);

    let mat_floor_handle = asset_manager.insert(
        MaterialData::solid(Vec3::new(0.08, 0.1, 0.15))
            .metallic(0.0)
            .roughness(0.2), // Damp floor
    );
    let mat_floor = MaterialHandle(mat_floor_handle);

    let mat_player_handle = asset_manager.insert(
        MaterialData::solid(Vec3::new(1.0, 0.2, 0.6))
            .metallic(0.8)
            .roughness(0.1), // Shiny metallic player
    );
    let mat_player = MaterialHandle(mat_player_handle);

    let color_palette = vec![
        Vec3::new(1.0, 0.9, 0.1), // Yellow
        Vec3::new(0.1, 1.0, 0.2), // Green
        Vec3::new(0.1, 0.5, 1.0), // Blue
        Vec3::new(1.0, 0.4, 0.0), // Orange
        Vec3::new(0.8, 0.1, 1.0), // Purple
        Vec3::new(0.0, 1.0, 1.0), // Cyan
    ];
    let mat_collectibles: Vec<MaterialHandle> = color_palette
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let metallic = if i < 3 { 1.0 } else { 0.0 };
            let roughness = ((i % 3) as f32 * 0.4 + 0.1).min(1.0);
            MaterialHandle(
                asset_manager.insert(MaterialData::solid(c).metallic(metallic).roughness(roughness)),
            )
        })
        .collect();

    let game_assets = GameAssets {
        mesh_handles: vec![mesh_sphere_handle, mesh_cube_handle],
        material_handles: std::iter::once(mat_floor_handle)
            .chain(std::iter::once(mat_player_handle))
            .chain(mat_collectibles.iter().map(|m| m.0))
            .collect(),
    };

    // AssetManager is not stored in World (HotReloadWatcher is !Sync); we keep it in the closure.
    world.insert_resource(game_assets);

    // --- IBL Setup ---
    // In a real app, you'd load this from a file.
    // We'll check if an assets folder exists, otherwise we use the dummy (black).
    let hdr_path = "assets/skybox.hdr";
    if std::path::Path::new(hdr_path).exists() {
        if let Ok(hdr_bytes) = std::fs::read(hdr_path) {
            match redox_render::resource::texture::Texture::from_hdr_bytes(
                render_ctx.device(),
                render_ctx.queue(),
                &hdr_bytes,
                "Skybox HDR",
            ) {
                Ok(hdr_tex) => {
                    log::info!("✅ Loaded environment HDR: {}", hdr_path);
                    render_ctx.set_environment(&hdr_tex);
                }
                Err(e) => log::error!("❌ Failed to parse HDR: {:?}", e),
            }
        }
    } else {
        log::warn!(
            "⚠️ No skybox.hdr found at {}. IBL will use black fallback.",
            hdr_path
        );
    }

    // --- Scene Setup ---

    // Floor
    let floor_entity = world.spawn();
    world.add_component(
        floor_entity,
        Transform {
            translation: Vec3::new(0.0, -1.0, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::new(20.0, 1.0, 20.0), // 40x40 area
        },
    );
    world.add_component(floor_entity, mesh_cube.clone());
    world.add_component(floor_entity, mat_floor.clone());

    // Floor physics
    let floor_rb = RigidBodyBuilder::fixed().translation(Vector::new(0.0, -1.0, 0.0));
    let floor_col = ColliderBuilder::cuboid(20.0, 0.5, 20.0);
    let physics = world.get_resource_mut::<PhysicsContext>().unwrap();
    let _ = physics.add_rigid_body(floor_entity, floor_rb, vec![floor_col]);

    // Player
    let player_entity = world.spawn();
    world.add_component(
        player_entity,
        Transform {
            translation: Vec3::new(0.0, 2.0, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::new(1.0, 1.0, 1.0),
        },
    );
    world.add_component(player_entity, mesh_sphere.clone());
    world.add_component(player_entity, mat_player.clone());
    world.add_component(
        player_entity,
        AudioEmitter {
            volume: 1.0,
            ..Default::default()
        },
    );

    // Player physics
    let player_rb = RigidBodyBuilder::dynamic()
        .translation(Vector::new(0.0, 2.0, 0.0))
        .linear_damping(1.0) // Add some drag so it doesn't roll forever
        .angular_damping(1.0);
    // Sphere collider matching radius 0.5
    let player_col = ColliderBuilder::ball(0.5).restitution(0.3);

    let physics = world.get_resource_mut::<PhysicsContext>().unwrap();
    let (player_rb_handle, _) = physics
        .add_rigid_body(player_entity, player_rb, vec![player_col])
        .unwrap();

    world.add_component(player_entity, Player(player_rb_handle));

    // Create a spawn helper using refs to the handles
    let spawn_cube_at = |w: &mut World,
                         mh: MeshHandle,
                         rgb_idx: usize,
                         mats: &[MaterialHandle],
                         m_colors: &[Vec3],
                         x,
                         z| {
        let ent = w.spawn();
        let color = m_colors[rgb_idx % m_colors.len()];
        w.add_component(
            ent,
            Transform {
                translation: Vec3::new(x, 0.5, z),
                rotation: Quat::IDENTITY,
                scale: Vec3::new(0.5, 0.5, 0.5),
            },
        );
        w.add_component(ent, Collectible);
        w.add_component(ent, mh);
        w.add_component(ent, mats[rgb_idx % mats.len()].clone());

        // Add a point light that matches the cube color
        w.add_component(
            ent,
            PointLight::new(
                Vec3::new(x, 1.0, z),
                color,
                2.0, // intensity
                5.0, // radius
            ),
        );

        let rb = RigidBodyBuilder::fixed().translation(Vector::new(x, 0.5, z));
        let col = ColliderBuilder::cuboid(0.25, 0.25, 0.25).sensor(true); // Sensor means no physical collision response
        let physics = w.get_resource_mut::<PhysicsContext>().unwrap();
        let _ = physics.add_rigid_body(ent, rb, vec![col]);
    };

    let mut rng = rand::thread_rng();
    for _ in 0..15 {
        let x = rng.gen_range(-15.0..15.0);
        let z = rng.gen_range(-15.0..15.0);
        let rgb_idx = rng.gen_range(0..mat_collectibles.len());
        spawn_cube_at(
            &mut world,
            mesh_cube.clone(),
            rgb_idx,
            &mat_collectibles,
            &color_palette,
            x,
            z,
        );
    }

    // --- Light ---
    let light_entity = world.spawn();
    world.add_component(
        light_entity,
        DirectionalLight {
            color: Vec3::new(1.0, 0.9, 0.8),
            intensity: 2.5,
            direction: Vec3::new(-0.5, -1.0, -0.5).normalize(),
        },
    );

    // --- Camera ---
    let camera_entity = world.spawn();
    let cam_pos = Vec3::new(0.0, 10.0, 15.0);
    let view_mat = Mat4::look_at_rh(cam_pos, Vec3::ZERO, Vec3::Y);
    world.add_component(
        camera_entity,
        Camera {
            fov_y: 45.0_f32.to_radians(),
            near: 0.1,
            far: 1000.0,
            aspect_ratio: 1280.0 / 720.0,
        },
    );
    world.add_component(
        camera_entity,
        Transform {
            translation: cam_pos,
            rotation: Quat::from_mat4(&view_mat.inverse()),
            scale: Vec3::ONE,
        },
    );
    // Add spatial listener to camera
    world.add_component(camera_entity, AudioListener::default());

    let mut start_time = Instant::now();
    // Last frame time, kept so the RedrawRequested handler can pass it to the
    // debug stats panel (which needs dt before building UI).
    let mut last_dt: f32 = 1.0 / 60.0;

    // --- Main Loop ---
    #[allow(deprecated)]
    let _ = event_loop.run(move |event, control_flow| {
        if let Event::WindowEvent { event: ref we, .. } = event {
            // Forward event to egui
            if ui_ctx.handle_window_event(&window, we) {
                return;
            }

            // Forward event to input state
            input_state.process_window_event(we);
        }

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => control_flow.exit(),

            Event::WindowEvent {
                event: WindowEvent::Resized(physical_size),
                ..
            } => {
                render_ctx.resize(physical_size.width, physical_size.height);
                if let Some(cam) = world.get_component_mut::<Camera>(camera_entity) {
                    cam.aspect_ratio = physical_size.width as f32 / physical_size.height as f32;
                }
            }

            Event::AboutToWait => {
                let now = Instant::now();
                let dt = now.duration_since(start_time).as_secs_f32();
                start_time = now;
                last_dt = dt;

                input_state.begin_frame();

                // --- Debug overlay hotkeys (F1 / F2 / F3) ---
                use winit::keyboard::KeyCode;
                if input_state.keyboard.just_pressed(KeyCode::F1) {
                    ui_ctx.debug.stats.open = !ui_ctx.debug.stats.open;
                }
                if input_state.keyboard.just_pressed(KeyCode::F2) {
                    ui_ctx.debug.inspector.open = !ui_ctx.debug.inspector.open;
                }
                if input_state.keyboard.just_pressed(KeyCode::F3) {
                    ui_ctx.show_debug = !ui_ctx.show_debug;
                }

                // --- 1. System: Player Input & Movement ---
                // Helper mapping actions to analog values
                let get_analog = |name| match actions.evaluate(
                    name,
                    &input_state.keyboard,
                    &input_state.mouse,
                ) {
                    redox_input::action::ActionKind::Analog(v) => v,
                    redox_input::action::ActionKind::Digital(b) => {
                        if b {
                            1.0
                        } else {
                            0.0
                        }
                    }
                };

                let move_fwd = get_analog("MoveForward");
                let move_bwd = get_analog("MoveBackward");
                let move_left = get_analog("MoveLeft");
                let move_right = get_analog("MoveRight");
                let reset_btn = get_analog("Reset");

                let mut force = Vec3::ZERO;
                force.z = move_bwd - move_fwd;
                force.x = move_right - move_left;

                if force.length_squared() > 0.0 {
                    force = force.normalize() * 30.0;
                }

                // Apply force to player
                let player_handle = world.get_component::<Player>(player_entity).unwrap().0;
                let physics = world.get_resource_mut::<PhysicsContext>().unwrap();

                if let Some(rb) = physics.rigid_bodies.get_mut(player_handle) {
                    rb.apply_impulse(Vector::new(force.x * dt, 0.0, force.z * dt), true);
                }

                // --- 2. System: Fetch updated player Transform ---

                // Step physics
                step_physics(&mut world, dt);
                sync_from_physics(&mut world);

                let player_pos = world
                    .get_component::<Transform>(player_entity)
                    .unwrap()
                    .translation;

                // --- 3. System: Camera Follow ---
                if let Some(cam_tf) = world.get_component_mut::<Transform>(camera_entity) {
                    let target_pos = player_pos + Vec3::new(0.0, 10.0, 10.0);
                    // Very simple interpolation
                    cam_tf.translation = cam_tf.translation.lerp(target_pos, 5.0 * dt);
                    let view_mat = Mat4::look_at_rh(cam_tf.translation, player_pos, Vec3::Y);
                    cam_tf.rotation = Quat::from_mat4(&view_mat.inverse());
                }

                // --- 4. System: Collectible Collisions ---
                let mut to_remove = Vec::new();

                // Using normal query (single component, so we just iterate entities manually)
                let entities: Vec<_> = world.all_entities().collect();
                for e in entities {
                    if world.get_component::<Collectible>(e).is_some() {
                        if let Some(col_tf) = world.get_component::<Transform>(e) {
                            // Simple distance check
                            let dist = player_pos.distance(col_tf.translation);
                            if dist < 1.0 {
                                // Sphere radius (0.5) + Cube half-diag (~0.4)
                                to_remove.push(e);

                                // Make a simple blip sound via spatial audio
                                if let Some(_audio) = world.get_resource_mut::<AudioContext>() {
                                    // A short built-in sine wave doesn't require loaded assets
                                    // but Kira lets us just load static files. However, to avoid requiring
                                    // local wave files, we'll try to just trigger something, or omit.
                                    // Wait, Kira doesn't have a synth oscillator built-in easily accessible
                                    // without custom implementations in v0.8. Let's just log it if we can't play right away.
                                }
                            }
                        }

                        // Rotate collectible
                        if let Some(tf) = world.get_component_mut::<Transform>(e) {
                            tf.rotation *= Quat::from_axis_angle(Vec3::Y, 2.0 * dt);
                        }
                    }
                }

                if !to_remove.is_empty() {
                    let score = world.get_resource_mut::<GameScore>().unwrap();
                    score.0 += to_remove.len() as u32;

                    {
                        let physics = world.get_resource_mut::<PhysicsContext>().unwrap();
                        for e in &to_remove {
                            let _ = physics.remove_rigid_body(*e); // Remove phys body
                        }
                    }

                    for e in &to_remove {
                        world.despawn(*e); // Remove from ECS
                    }

                    for _ in 0..to_remove.len() {
                        // Respawn a new one to keep game going
                        let x = rng.gen_range(-15.0..15.0);
                        let z = rng.gen_range(-15.0..15.0);
                        let rgb_idx = rng.gen_range(0..mat_collectibles.len());
                        spawn_cube_at(
                            &mut world,
                            mesh_cube.clone(),
                            rgb_idx,
                            &mat_collectibles,
                            &color_palette,
                            x,
                            z,
                        );
                    }
                }

                // Reset Logic
                if reset_btn > 0.5 {
                    let score = world.get_resource_mut::<GameScore>().unwrap();
                    score.0 = 0;

                    let physics = world.get_resource_mut::<PhysicsContext>().unwrap();
                    if let Some(rb) = physics.rigid_bodies.get_mut(player_handle) {
                        rb.set_translation(Vector::new(0.0, 2.0, 0.0), true);
                        rb.set_linvel(Vector::zeros(), true);
                        rb.set_angvel(Vector::zeros(), true);
                    }
                }

                window.request_redraw();
            }

            Event::WindowEvent {
                event: WindowEvent::RedrawRequested,
                ..
            } => {
                // --- Asset manager update (process completed loads) ---
                asset_manager.update(&mut world);
                // --- Sync assets to render context (upload ready meshes/materials to GPU) ---
                if let Some(game_assets) = world.get_resource::<GameAssets>() {
                    sync_assets_to_render(
                        &mut render_ctx,
                        &asset_manager,
                        &game_assets.mesh_handles,
                        &[],
                        &game_assets.material_handles,
                    );
                }

                // --- Render Prep ---
                // Sync Camera Uniform
                if let Some(cam) = world.get_component::<Camera>(camera_entity) {
                    let tf = world.get_component::<Transform>(camera_entity).unwrap();
                    let view = Mat4::from_quat(tf.rotation).inverse()
                        * Mat4::from_translation(-tf.translation);
                    let proj = cam.projection_matrix();
                    render_ctx.camera_uniform.view_proj = (proj * view).to_cols_array_2d();
                    render_ctx.camera_uniform.camera_pos =
                        [tf.translation.x, tf.translation.y, tf.translation.z, 1.0];
                    render_ctx.update_camera_buffer();
                }

                // Sync Light Uniform
                if let Some(light) = world.get_component::<DirectionalLight>(light_entity) {
                    let mut light_u = LightUniform {
                        dir_direction: [
                            light.direction.x,
                            light.direction.y,
                            light.direction.z,
                            0.0,
                        ],
                        dir_color: [light.color.x, light.color.y, light.color.z, light.intensity],
                        ambient: [0.1, 0.1, 0.15, 1.0], // Darker background ambient to let point lights shine
                        ..Default::default()
                    };

                    // Collect point lights (limit to 8)
                    let entities = world.all_entities();
                    let mut point_lights = Vec::new();
                    for e in entities {
                        if let Some(pl) = world.get_component::<PointLight>(e) {
                            // Update position from transform if available
                            let mut pl_cloned = pl.clone();
                            if let Some(tf) = world.get_component::<Transform>(e) {
                                pl_cloned.position = tf.translation;
                            }
                            light_u.add_point_light(&pl_cloned);
                            point_lights.push(pl_cloned);
                        }
                    }

                    render_ctx.update_light_buffer(&light_u);
                    
                    // Update cluster lights for better performance with many lights
                    if let Some(cam) = world.get_component::<Camera>(camera_entity) {
                        render_ctx.update_cluster_lights(&point_lights, &cam);
                    }
                }

                // Build Render Objects list using the new system
                let render_objects = extract_render_objects(&world, &render_ctx);

                // --- Render Execution ---
                let output = render_ctx.surface().get_current_texture().unwrap();
                let surface_view = output
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());

                let mut encoder =
                    render_ctx
                        .device()
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Main Encoder"),
                        });

                // 0. Update Shadow Matrix
                let shadow_matrix = {
                    if let Some(light) = world.get_component::<DirectionalLight>(light_entity) {
                        let light_dir = light.direction;
                        let size = 25.0;
                        let proj = redox_math::orthographic(-size, size, -size, size, -50.0, 50.0);
                        let view = redox_math::look_at(
                            light_dir * -20.0,
                            redox_math::Vec3::ZERO,
                            redox_math::Vec3::Y,
                        );
                        proj * view
                    } else {
                        redox_math::Mat4::IDENTITY
                    }
                };

                // Update light uniform with shadow matrix
                if let Some(light) = world.get_component::<DirectionalLight>(light_entity) {
                    let mut light_u = LightUniform {
                        dir_direction: [
                            light.direction.x,
                            light.direction.y,
                            light.direction.z,
                            0.0,
                        ],
                        dir_color: [light.color.x, light.color.y, light.color.z, light.intensity],
                        ambient: [0.1, 0.1, 0.15, 1.0],
                        shadow_view_proj: shadow_matrix.to_cols_array_2d(),
                        ..Default::default()
                    };

                    let entities = world.all_entities();
                    let mut point_lights = Vec::new();
                    for e in entities {
                        if let Some(pl) = world.get_component::<PointLight>(e) {
                            let mut pl_cloned = pl.clone();
                            if let Some(tf) = world.get_component::<Transform>(e) {
                                pl_cloned.position = tf.translation;
                            }
                            light_u.add_point_light(&pl_cloned);
                            point_lights.push(pl_cloned);
                        }
                    }
                    render_ctx.update_light_buffer(&light_u);
                    
                    // Update cluster lights for better performance with many lights
                    if let Some(cam) = world.get_component::<Camera>(camera_entity) {
                        render_ctx.update_cluster_lights(&point_lights, &cam);
                    }
                }

                // Sync all GPU buffers before passes
                render_ctx.update_model_buffer(&render_objects);

                // 1. Shadow Pass
                {
                    let shadow_view_proj_buffer =
                        redox_render::resource::buffer::create_uniform_buffer(
                            render_ctx.device(),
                            "shadow_matrix_temp",
                            bytemuck::bytes_of(&shadow_matrix.to_cols_array_2d()),
                        );
                    let shadow_matrix_bg =
                        render_ctx
                            .device()
                            .create_bind_group(&wgpu::BindGroupDescriptor {
                                label: Some("shadow_matrix_bg"),
                                layout: &render_ctx.shadow_pass.bind_group_layout,
                                entries: &[wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: shadow_view_proj_buffer.as_entire_binding(),
                                }],
                            });

                    let mut s_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("shadow_pass"),
                        color_attachments: &[],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &render_ctx.shadow_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.0),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });

                    s_pass.set_pipeline(&render_ctx.shadow_pass.pipeline);
                    s_pass.set_bind_group(0, &shadow_matrix_bg, &[]);
                    s_pass.set_bind_group(1, &render_ctx.shadow_model_bind_group, &[]);

                    for (i, obj) in render_objects.iter().enumerate() {
                        if let Some(gpu_mesh) = render_ctx.meshes.get(obj.mesh_index) {
                            s_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                            s_pass.set_index_buffer(
                                gpu_mesh.index_buffer.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            s_pass.draw_indexed(
                                0..gpu_mesh.index_count,
                                0,
                                (i as u32)..(i as u32 + 1),
                            );
                        }
                    }
                }

                // 2. Normal pass (normals + linear depth for SSAO)
                {
                    let mut normal_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("normal_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &render_ctx.normal_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &render_ctx.depth_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.0),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        ..Default::default()
                    });
                    normal_pass.set_pipeline(&render_ctx.normal_pass.pipeline);
                    normal_pass.set_bind_group(0, &render_ctx.normal_bind_group, &[]);
                    for (i, obj) in render_objects.iter().enumerate() {
                        if let Some(gpu_mesh) = render_ctx.meshes.get(obj.mesh_index) {
                            normal_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                            normal_pass.set_index_buffer(gpu_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                            normal_pass.draw_indexed(0..gpu_mesh.index_count, 0, (i as u32)..(i as u32 + 1));
                        }
                    }
                }

                // 3. SSAO pass
                {
                    let mut ssao_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("ssao_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &render_ctx.ssao_raw_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        ..Default::default()
                    });
                    ssao_pass.set_pipeline(&render_ctx.ssao_pass.pipeline);
                    ssao_pass.set_bind_group(0, &render_ctx.ssao_bind_group, &[]);
                    ssao_pass.draw(0..3, 0..1);
                }

                // 4. SSAO blur pass
                {
                    let mut blur_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("ssao_blur_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &render_ctx.ssao_blurred_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        ..Default::default()
                    });
                    blur_pass.set_pipeline(&render_ctx.ssao_pass.blur_pipeline);
                    blur_pass.set_bind_group(0, &render_ctx.ssao_blur_bind_group, &[]);
                    blur_pass.draw(0..3, 0..1);
                }

                // 5. Scene Pass (into HDR)
                {
                    let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("scene_pass_hdr"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &render_ctx.hdr_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: 0.05,
                                    g: 0.05,
                                    b: 0.08,
                                    a: 1.0,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &render_ctx.depth_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.0),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        ..Default::default()
                    });
                    render_ctx.record_draw(&mut rpass, &render_objects);
                }

                // 6. Post-process (into Surface)
                {
                    let mut final_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("final_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &surface_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        ..Default::default()
                    });

                    // Tone mapping fullscreen triangle
                    final_pass.set_pipeline(&render_ctx.tone_mapping_pipeline);
                    final_pass.set_bind_group(0, &render_ctx.tone_mapping_bind_group, &[]);
                    final_pass.draw(0..3, 0..1);
                }

                // 4. UI Overlay (into Surface)
                // Record frame timing before building UI (uses last frame's dt
                // so it is available immediately after the first frame).
                ui_ctx.record_frame(last_dt);

                ui_ctx.begin_frame(&window);

                let score = world.get_resource::<GameScore>().unwrap().0;
                egui::Window::new("RedOx Engine")
                    .anchor(egui::Align2::LEFT_TOP, [10.0, 10.0])
                    .show(&ui_ctx.egui_ctx, |ui| {
                        ui.heading("PBR + Shadows + HDR + Tone Mapping");
                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.label("Score: ");
                            ui.label(
                                egui::RichText::new(score.to_string())
                                    .color(Color32::from_rgb(255, 100, 200))
                                    .strong()
                                    .size(28.0),
                            );
                        });
                        ui.separator();
                        ui.label("Controls: WASD to move ball, R to reset.");
                        ui.separator();
                        ui.weak("F1 Stats · F2 Inspector · F3 Toggle Debug");
                    });

                // Draw the debug overlay (stats, inspector, control bar).
                // The overlay reads `world` for the entity inspector.
                ui_ctx.draw_debug(&world);

                let screen_desc = egui_wgpu::ScreenDescriptor {
                    size_in_pixels: [render_ctx.config().width, render_ctx.config().height],
                    pixels_per_point: window.scale_factor() as f32,
                };

                ui_ctx.end_frame_and_render(
                    render_ctx.device(),
                    render_ctx.queue(),
                    &mut encoder,
                    &window,
                    &surface_view,
                    screen_desc,
                );

                render_ctx.queue().submit(std::iter::once(encoder.finish()));
                output.present();
            }

            _ => {}
        }
    });
}
