//! Crystal Forest Demo — 10 000 кристаллов с анимацией и цветными огнями.

use redox_core::app::AppBuilder;
use redox_core::config::EngineConfig;
use redox_core::dispatcher::Stage;
use redox_ecs::world::World;
use redox_math::{Quat, Vec3};
use redox_render::camera::Camera;
use redox_render::context::RenderContext;
use redox_render::light::{DirectionalLight, LightUniform, PointLight};
use redox_render::material::Material;
use redox_render::mesh::primitive::{create_cube, create_sphere, create_torus};
use redox_render::systems::{MaterialHandle, MeshHandle, RenderObject, Transform};

// ---------------------------------------------------------------------------
// Ресурс сцены
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Crystal {
    transform: Transform,
    mesh_handle: MeshHandle,
    material_handle: MaterialHandle,
}

#[derive(Clone)]
struct AnimLight {
    light: PointLight,
    id: u32,
}

struct ForestScene {
    crystals: Vec<Crystal>,
    lights: Vec<AnimLight>,
    camera_pos: Vec3,
    camera_rot: Quat,
}

// ---------------------------------------------------------------------------
// Инициализация сцены
// ---------------------------------------------------------------------------

fn build_scene(ctx: &mut RenderContext) -> ForestScene {
    // Меши
    let mesh_handles = [
        MeshHandle(ctx.upload_mesh(&create_cube())),
        MeshHandle(ctx.upload_mesh(&create_sphere(1.0, 16))),
        MeshHandle(ctx.upload_mesh(&create_torus(0.6, 0.2, 16, 8))),
    ];

    // Материалы
    let colors = [
        Vec3::new(0.4, 0.8, 1.0),
        Vec3::new(1.0, 0.4, 0.8),
        Vec3::new(0.4, 1.0, 0.4),
        Vec3::new(0.8, 0.4, 1.0),
        Vec3::new(1.0, 0.8, 0.4),
    ];
    let mat_handles: Vec<MaterialHandle> = colors
        .iter()
        .map(|&c| MaterialHandle(ctx.add_material(Material::solid(c))))
        .collect();

    // 10 000 кристаллов (детерминированное размещение)
    let mut crystals = Vec::with_capacity(10_000);
    for i in 0u32..10_000 {
        let fi = i as f32;
        let x = (fi * 0.1).sin() * 60.0;
        let z = (fi * 0.13).cos() * 60.0;
        let y = (fi * 0.07).sin().abs() * 2.5 + 0.5;
        let scale = 0.3 + (fi * 0.11).sin().abs() * 0.8;

        crystals.push(Crystal {
            transform: Transform {
                translation: Vec3::new(x, y, z),
                rotation: Quat::from_rotation_y(fi * 0.618),
                scale: Vec3::new(scale * 0.5, scale, scale * 0.5),
            },
            mesh_handle: mesh_handles[(i as usize) % mesh_handles.len()],
            material_handle: mat_handles[(i as usize) % mat_handles.len()],
        });
    }

    // 5 цветных огней
    let light_colors = [
        Vec3::new(1.0, 0.2, 0.2),
        Vec3::new(0.2, 1.0, 0.2),
        Vec3::new(0.2, 0.4, 1.0),
        Vec3::new(1.0, 0.2, 1.0),
        Vec3::new(1.0, 0.6, 0.1),
    ];
    let lights = light_colors
        .iter()
        .enumerate()
        .map(|(id, &color)| AnimLight {
            light: PointLight::new(Vec3::ZERO, color, 2.0, 20.0),
            id: id as u32,
        })
        .collect();

    ForestScene {
        crystals,
        lights,
        camera_pos: Vec3::new(0.0, 15.0, 50.0),
        camera_rot: Quat::IDENTITY,
    }
}

// Вычисляет ориентацию камеры, смотрящей из `eye` в направлении `target`.
fn look_toward(eye: Vec3, target: Vec3) -> Quat {
    let forward = (target - eye).normalize();
    // Стандартная forward-ось камеры — -Z
    let default_fwd = Vec3::new(0.0, 0.0, -1.0);
    Quat::from_rotation_arc(default_fwd, forward)
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() {
    let config = EngineConfig {
        window_title: "RedOx: Magic Crystal Forest — 10 000 instances".to_string(),
        window_width: 1600,
        window_height: 900,
        ..Default::default()
    };

    let mut initialized = false;

    AppBuilder::new(config)
        // ------------------------------------------------------------------
        // Update: анимация сцены
        // ------------------------------------------------------------------
        .add_system(
            Stage::Update,
            move |world: &mut World, ctx: &mut RenderContext| {
                // Инициализация при первом кадре
                if !initialized {
                    initialized = true;

                    let scene = build_scene(ctx);
                    world.insert_resource(scene);

                    world.insert_resource(Camera::new(
                        60.0_f32.to_radians(),
                        1600.0 / 900.0,
                        0.1,
                        1000.0,
                    ));

                    world.insert_resource(DirectionalLight::new(
                        Vec3::new(-0.3, 1.0, 0.5),
                        Vec3::new(0.6, 0.7, 1.0),
                        0.5,
                    ));
                }

                let (total_time, delta_time) = world
                    .get_resource::<redox_core::time::Time>()
                    .map(|t| (t.total_time, t.delta_time))
                    .unwrap_or((0.0, 0.016));

                if let Some(scene) = world.get_resource_mut::<ForestScene>() {
                    // Орбита камеры
                    let r = 55.0_f32;
                    let cam_speed = 0.12;
                    scene.camera_pos = Vec3::new(
                        (total_time * cam_speed).cos() * r,
                        15.0 + (total_time * 0.07).sin() * 8.0,
                        (total_time * cam_speed).sin() * r,
                    );
                    scene.camera_rot = look_toward(scene.camera_pos, Vec3::ZERO);

                    // Анимация огней
                    for anim in &mut scene.lights {
                        let fi = anim.id as f32;
                        let offset = fi * std::f32::consts::TAU / 5.0;
                        let light_r = 25.0 + fi * 3.0;
                        let speed = 0.4 + fi * 0.08;
                        anim.light.position = Vec3::new(
                            (total_time * speed + offset).cos() * light_r,
                            3.0 + (total_time * 1.5 + offset).sin() * 2.5,
                            (total_time * speed + offset).sin() * light_r,
                        );
                        anim.light.intensity = 1.5 + (total_time * 2.0 + offset).sin() * 0.5;
                    }

                    // Вращение кристаллов
                    for crystal in &mut scene.crystals {
                        crystal.transform.rotation =
                            crystal.transform.rotation * Quat::from_rotation_y(0.3 * delta_time);
                    }
                }
            },
        )
        // ------------------------------------------------------------------
        // Render: подготовка и рисование кадра
        // ------------------------------------------------------------------
        .add_system(
            Stage::Render,
            |world: &mut World, ctx: &mut RenderContext| {
                let total_time = world
                    .get_resource::<redox_core::time::Time>()
                    .map(|t| t.total_time)
                    .unwrap_or(0.0);

                // Обновить камеру
                let (cam_pos, cam_rot) = world
                    .get_resource::<ForestScene>()
                    .map(|s| (s.camera_pos, s.camera_rot))
                    .unwrap_or((Vec3::new(0.0, 15.0, 50.0), Quat::IDENTITY));

                if let Some(camera) = world.get_resource::<Camera>() {
                    ctx.camera_uniform.update(camera, cam_pos, cam_rot);
                }
                ctx.update_camera_buffer();

                // Обновить свет
                let dir_light = world
                    .get_resource::<DirectionalLight>()
                    .cloned()
                    .unwrap_or_default();
                let mut light_uni = LightUniform::new(&dir_light, Vec3::splat(0.04));
                if let Some(scene) = world.get_resource::<ForestScene>() {
                    for anim in &scene.lights {
                        light_uni.add_point_light(&anim.light);
                    }
                }
                ctx.update_light_buffer(&light_uni);

                // Собрать и нарисовать объекты
                let mut objects: Vec<RenderObject> = Vec::new();
                if let Some(scene) = world.get_resource::<ForestScene>() {
                    for crystal in &scene.crystals {
                        let glow = (total_time * 0.3 + crystal.transform.translation.x * 0.02)
                            .sin()
                            * 0.08
                            + 0.92;
                        objects.push(RenderObject {
                            model_matrix: crystal.transform.matrix(),
                            color: [glow, glow, glow, 1.0],
                            mesh_index: crystal.mesh_handle.0,
                            material_index: crystal.material_handle.0,
                        });
                    }
                }

                match ctx.render_frame(&objects) {
                    Ok(_) => {}
                    Err(wgpu::SurfaceError::Lost) => {
                        ctx.resize(ctx.config.width, ctx.config.height)
                    }
                    Err(wgpu::SurfaceError::OutOfMemory) => std::process::exit(1),
                    Err(e) => log::error!("Render error: {:?}", e),
                }
            },
        )
        .run();
}
