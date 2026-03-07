use std::sync::Arc;
use std::time::Instant;
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};

use redox_core::time::Time;
use redox_ecs::world::World;
// ЕДИНЫЙ ТРАНСФОРМ:
use redox_math::{Mat4, Quat, Transform, Vec3};

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_physics::components::{Collider, RigidBody};
use redox_physics::context::PhysicsContext;
use redox_physics::sync::{step_physics, sync_from_physics, sync_to_physics};
use redox_physics::utils::vec3_to_rapier;

use redox_render::camera::Camera;
use redox_render::context::RenderContext;
use redox_render::light::{DirectionalLight, LightUniform};
use redox_render::material::Material;
use redox_render::mesh::primitive::{create_cube, create_sphere};
use redox_render::systems::{MaterialHandle, MeshHandle, RenderObject};

fn main() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();
    pollster::block_on(run());
}

async fn run() {
    log::info!("🚀 Starting RedOx Unified Architecture Showcase...");

    let event_loop = EventLoop::new().unwrap();
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("RedOx Engine - Clean Architecture")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0))
            .build(&event_loop)
            .unwrap(),
    );

    let mut render_ctx = RenderContext::new(window.clone()).await;
    let mut world = World::new();
    let mut time = Time::new(1.0 / 60.0);

    // --- РЕСУРСЫ ---
    // Теперь физика живет внутри мира!
    world.insert_resource(PhysicsContext::new(Vec3::new(0.0, -15.0, 0.0)));

    // --- Ассеты ---
    let mesh_cube = MeshHandle(render_ctx.upload_mesh(&create_cube()));
    let mesh_sphere = MeshHandle(render_ctx.upload_mesh(&create_sphere(1.0, 32)));

    let mat_floor =
        MaterialHandle(render_ctx.add_material(Material::solid(Vec3::new(0.2, 0.2, 0.2))));
    let mat_wall =
        MaterialHandle(render_ctx.add_material(Material::solid(Vec3::new(0.3, 0.5, 0.7))));
    let mat_ball =
        MaterialHandle(render_ctx.add_material(Material::solid(Vec3::new(0.9, 0.1, 0.1))));

    // --- Свет ---
    let light_entity = world.spawn();
    world.add_component(
        light_entity,
        DirectionalLight {
            color: Vec3::new(1.0, 1.0, 1.0),
            intensity: 2.0,
            direction: Vec3::new(-0.5, -1.0, -0.5).normalize(),
        },
    );

    // --- Камера ---
    let camera_entity = world.spawn();
    let cam_pos = Vec3::new(25.0, 25.0, 25.0);
    let cam_target = Vec3::new(0.0, 5.0, 0.0);
    let view_mat = Mat4::look_at_rh(cam_pos, cam_target, Vec3::Y);

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

    let mut all_entities = Vec::new();

    // --- Создание Коробки ---
    let mut spawn_box = |pos: Vec3, scale: Vec3, mat: MaterialHandle| {
        let entity = world.spawn();
        world.add_component(
            entity,
            Transform {
                translation: pos,
                rotation: Quat::IDENTITY,
                scale,
            },
        );
        world.add_component(entity, mesh_cube.clone());
        world.add_component(entity, mat);

        let rb = RigidBodyBuilder::fixed().translation(vec3_to_rapier(pos));
        let col = ColliderBuilder::cuboid(scale.x / 2.0, scale.y / 2.0, scale.z / 2.0);

        let phys = world.get_resource_mut::<PhysicsContext>().unwrap();
        let (rb_h, col_h) = phys.add_rigid_body(entity, rb, vec![col]).unwrap();

        world.add_component(entity, RigidBody { handle: rb_h });
        world.add_component(entity, Collider { handle: col_h[0] });
        all_entities.push(entity);
    };

    spawn_box(
        Vec3::new(0.0, -0.5, 0.0),
        Vec3::new(20.0, 1.0, 20.0),
        mat_floor,
    );
    spawn_box(
        Vec3::new(-10.0, 4.0, 0.0),
        Vec3::new(1.0, 8.0, 20.0),
        mat_wall.clone(),
    );
    spawn_box(
        Vec3::new(10.0, 4.0, 0.0),
        Vec3::new(1.0, 8.0, 20.0),
        mat_wall.clone(),
    );
    spawn_box(
        Vec3::new(0.0, 4.0, -10.0),
        Vec3::new(20.0, 8.0, 1.0),
        mat_wall.clone(),
    );
    spawn_box(
        Vec3::new(0.0, 4.0, 10.0),
        Vec3::new(20.0, 8.0, 1.0),
        mat_wall,
    );

    // --- Спавн Шариков ---
    for y in 0..4 {
        for x in -2..=2 {
            for z in -2..=2 {
                let entity = world.spawn();
                let pos = Vec3::new(x as f32 * 2.0, 10.0 + (y as f32 * 2.0), z as f32 * 2.0);

                world.add_component(
                    entity,
                    Transform {
                        translation: pos,
                        rotation: Quat::IDENTITY,
                        scale: Vec3::splat(1.0),
                    },
                );
                world.add_component(entity, mesh_sphere.clone());
                world.add_component(entity, mat_ball.clone());

                let rb = RigidBodyBuilder::dynamic().translation(vec3_to_rapier(pos));
                let col = ColliderBuilder::ball(0.5).restitution(0.7);

                let phys = world.get_resource_mut::<PhysicsContext>().unwrap();
                let (rb_h, col_h) = phys.add_rigid_body(entity, rb, vec![col]).unwrap();

                world.add_component(entity, RigidBody { handle: rb_h });
                world.add_component(entity, Collider { handle: col_h[0] });
                all_entities.push(entity);
            }
        }
    }

    let mut last_frame_time = Instant::now();
    let mut minimized = false;

    event_loop
        .run(move |event, target| {
            target.set_control_flow(ControlFlow::Poll);

            match event {
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                } => target.exit(),
                Event::WindowEvent {
                    event: WindowEvent::Resized(size),
                    ..
                } => {
                    if size.width > 0 && size.height > 0 {
                        minimized = false;
                        render_ctx.resize(size.width, size.height);
                        if let Some(cam) = world.get_component_mut::<Camera>(camera_entity) {
                            cam.aspect_ratio = size.width as f32 / size.height as f32;
                        }
                    } else {
                        minimized = true;
                    }
                }
                Event::AboutToWait => {
                    if !minimized {
                        window.request_redraw();
                    }
                }
                Event::WindowEvent {
                    event: WindowEvent::RedrawRequested,
                    ..
                } => {
                    if minimized {
                        return;
                    }
                    let dt = last_frame_time.elapsed().as_secs_f32();
                    last_frame_time = Instant::now();
                    time.tick(dt);

                    // --- ЧИСТАЯ АРХИТЕКТУРА СИСТЕМ ---
                    while time.should_step_fixed() {
                        sync_to_physics(&mut world);
                        step_physics(&mut world, time.fixed_delta_time);
                        sync_from_physics(&mut world);
                        time.consume_fixed_step();
                    }

                    // --- РЕНДЕР ---
                    if let (Some(cam), Some(tf)) = (
                        world.get_component::<Camera>(camera_entity),
                        world.get_component::<Transform>(camera_entity),
                    ) {
                        render_ctx
                            .camera_uniform
                            .update(cam, tf.translation, tf.rotation);
                        render_ctx.update_camera_buffer();
                    }

                    if let Some(light) = world.get_component::<DirectionalLight>(light_entity) {
                        render_ctx.update_light_buffer(&LightUniform::new(light, Vec3::splat(0.2)));
                    }

                    let mut render_objects = Vec::new();
                    for &entity in &all_entities {
                        if let (Some(tf), Some(mesh), Some(mat)) = (
                            world.get_component::<Transform>(entity),
                            world.get_component::<MeshHandle>(entity),
                            world.get_component::<MaterialHandle>(entity),
                        ) {
                            render_objects.push(RenderObject {
                                model_matrix: tf.matrix(),
                                mesh_index: mesh.0,
                                material_index: mat.0,
                            });
                        }
                    }

                    match render_ctx.render_frame(&render_objects) {
                        Ok(_) => {}
                        Err(wgpu::SurfaceError::Lost) | Err(wgpu::SurfaceError::Outdated) => {
                            let size = window.inner_size();
                            if size.width > 0 && size.height > 0 {
                                render_ctx.resize(size.width, size.height);
                            }
                        }
                        Err(e) => eprintln!("Render Error: {:?}", e),
                    }
                }
                _ => {}
            }
        })
        .unwrap();
}
