use std::sync::Arc;
use std::time::Instant;
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
    keyboard::KeyCode,
};

use redox_core::time::Time;
use redox_ecs::world::World;
use redox_math::{Mat4, Quat, Vec3, Transform};
use redox_render::{
    context::RenderContext,
    camera::Camera,
    light::{DirectionalLight, LightUniform},
    material::Material,
    mesh::primitive::{create_cube, create_sphere},
    systems::{MaterialHandle, MeshHandle, RenderObject},
};
use redox_input::{
    state::InputState,
    action::{ActionMap, ActionBinding},
};
use redox_ui::{
    context::UiContext,
    debug::{stats::show_stats, inspector::show_inspector},
};

fn main() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();
    pollster::block_on(run());
}

async fn run() {
    log::info!("🚀 Starting RedOx Engine Full Demo...");

    let event_loop = EventLoop::new().unwrap();
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("RedOx Engine - Full Subsystems Demo")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0))
            .build(&event_loop)
            .unwrap(),
    );

    let mut render_ctx = RenderContext::new(window.clone()).await;
    let mut ui_ctx = UiContext::new(&window, render_ctx.device(), render_ctx.surface_format());
    
    let mut world = World::new();
    let mut time = Time::new(1.0 / 60.0);
    let mut input = InputState::new();

    // Setup input actions
    input.actions.add("MoveForward", ActionBinding::Key(KeyCode::KeyW));
    input.actions.add("MoveBackward", ActionBinding::Key(KeyCode::KeyS));
    input.actions.add("MoveLeft", ActionBinding::Key(KeyCode::KeyA));
    input.actions.add("MoveRight", ActionBinding::Key(KeyCode::KeyD));
    input.actions.add("ToggleDebug", ActionBinding::Key(KeyCode::F3));

    // Upload assets
    let mesh_cube = MeshHandle(render_ctx.upload_mesh(&create_cube()));
    let mesh_sphere = MeshHandle(render_ctx.upload_mesh(&create_sphere(1.0, 32)));
    
    let mat_floor = MaterialHandle(render_ctx.add_material(Material::solid(Vec3::new(0.2, 0.2, 0.2))));
    let mat_box = MaterialHandle(render_ctx.add_material(Material::solid(Vec3::new(0.8, 0.4, 0.1))));

    // Spawn camera
    let camera_entity = world.spawn();
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
            translation: Vec3::new(0.0, 5.0, 15.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        },
    );

    // Spawn light
    let light_entity = world.spawn();
    world.add_component(
        light_entity,
        DirectionalLight {
            color: Vec3::new(1.0, 1.0, 0.9),
            intensity: 1.5,
            direction: Vec3::new(-1.0, -1.0, -1.0).normalize(),
        },
    );

    // Spawn floor
    let floor_entity = world.spawn();
    world.add_component(
        floor_entity,
        Transform {
            translation: Vec3::new(0.0, -1.0, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::new(20.0, 1.0, 20.0),
        },
    );
    world.add_component(floor_entity, mesh_cube.clone());
    world.add_component(floor_entity, mat_floor);

    // Spawn rotating box
    let box_entity = world.spawn();
    world.add_component(
        box_entity,
        Transform {
            translation: Vec3::new(0.0, 1.0, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::splat(2.0),
        },
    );
    world.add_component(box_entity, mesh_cube.clone());
    world.add_component(box_entity, mat_box);

    let mut last_frame_time = Instant::now();
    let mut display_debug = true;

    event_loop
        .run(move |event, target| {
            target.set_control_flow(ControlFlow::Poll);

            // Let EgUI handle events first
            if let Event::WindowEvent { event: ref window_event, .. } = event {
                if ui_ctx.handle_window_event(&window, window_event) {
                    return;
                }
                
                // Track input
                input.process_window_event(window_event);
            }

            match event {
                Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                    target.exit();
                }
                Event::WindowEvent { event: WindowEvent::Resized(size), .. } => {
                    if size.width > 0 && size.height > 0 {
                        render_ctx.resize(size.width, size.height);
                        if let Some(cam) = world.get_component_mut::<Camera>(camera_entity) {
                            cam.aspect_ratio = size.width as f32 / size.height as f32;
                        }
                    }
                }
                Event::AboutToWait => {
                    window.request_redraw();
                }
                Event::WindowEvent { event: WindowEvent::RedrawRequested, .. } => {
                    let dt = last_frame_time.elapsed().as_secs_f32();
                    last_frame_time = Instant::now();
                    time.tick(dt);
                    
                    input.begin_frame();

                    // Input logic
                    if input.keyboard.just_pressed(KeyCode::F3) {
                        display_debug = !display_debug;
                    }

                    // Camera movement logic
                    if let Some(cam_tf) = world.get_component_mut::<Transform>(camera_entity) {
                        let speed = 10.0 * dt;
                        if input.action_active("MoveForward") { cam_tf.translation.z -= speed; }
                        if input.action_active("MoveBackward") { cam_tf.translation.z += speed; }
                        if input.action_active("MoveLeft") { cam_tf.translation.x -= speed; }
                        if input.action_active("MoveRight") { cam_tf.translation.x += speed; }
                    }

                    // Animation logic
                    if let Some(box_tf) = world.get_component_mut::<Transform>(box_entity) {
                        let rot = Quat::from_rotation_y(time.total_time);
                        let rot_z = Quat::from_rotation_z(time.total_time * 0.5);
                        box_tf.rotation = rot * rot_z;
                    }

                    // Rendering Prep
                    if let (Some(cam), Some(tf)) = (
                        world.get_component::<Camera>(camera_entity),
                        world.get_component::<Transform>(camera_entity),
                    ) {
                        render_ctx.camera_uniform.update(cam, tf.translation, tf.rotation);
                        render_ctx.update_camera_buffer();
                    }

                    if let Some(light) = world.get_component::<DirectionalLight>(light_entity) {
                        render_ctx.update_light_buffer(&LightUniform::new(light, Vec3::splat(0.2)));
                    }

                    let mut render_objects = Vec::new();
                    for entity in world.all_entities() {
                        if let (Some(transform), Some(mesh), Some(mat)) = (
                            world.get_component::<Transform>(entity),
                            world.get_component::<MeshHandle>(entity),
                            world.get_component::<MaterialHandle>(entity),
                        ) {
                            render_objects.push(RenderObject {
                                model_matrix: transform.matrix(),
                                color: [1.0, 1.0, 1.0, 1.0],
                                mesh_index: mesh.0,
                                material_index: mat.0,
                            });
                        }
                    }

                    // UI Rendering
                    ui_ctx.begin_frame(&window);
                    
                    if display_debug {
                        show_stats(ui_ctx.ctx(), dt, time.total_time, world.all_entities().count());
                        show_inspector(ui_ctx.ctx(), &world);
                    }

                    // Present frame
                    match render_ctx.surface.get_current_texture() {
                        Ok(output) => {
                            let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
                            let mut encoder = render_ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Render Encoder") });

                            // 1. Draw 3D scene
                            {
                                let mut rpass = render_ctx.begin_pass(&mut encoder, &view);
                                render_ctx.record_draw(&mut rpass, &render_objects);
                            }

                            // 2. Draw UI
                            let screen_desc = egui_wgpu::ScreenDescriptor {
                                size_in_pixels: [render_ctx.config().width, render_ctx.config().height],
                                pixels_per_point: window.scale_factor() as f32,
                            };
                            
                            ui_ctx.end_frame_and_render(
                                render_ctx.device(),
                                render_ctx.queue(),
                                &mut encoder,
                                &window,
                                &view,
                                screen_desc,
                            );

                            render_ctx.queue().submit(std::iter::once(encoder.finish()));
                            output.present();
                        }
                        Err(e) => eprintln!("Render error: {:?}", e),
                    }
                }
                _ => {}
            }
        })
        .unwrap();
}
