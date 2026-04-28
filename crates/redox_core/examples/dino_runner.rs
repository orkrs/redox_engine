use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use redox_core::app::AppBuilder;
use redox_core::config::EngineConfig;
use redox_core::dispatcher::Stage;
use redox_ecs::{Entity, World};
use redox_input::InputState;
use redox_math::{Quat, Vec3};
use redox_render::camera::Camera;
use redox_render::context::RenderContext;
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::systems::{RenderObject, Transform};
use winit::keyboard::KeyCode;

#[derive(Clone, Copy)]
struct Renderable {
    mesh_index: usize,
    material_index: usize,
}

#[derive(Clone)]
struct CactusInstance {
    parts: Vec<Entity>,
    local_offsets: Vec<Vec3>,
    position: Vec3,
    half_extents: Vec3,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RunPhase {
    Running,
    GameOver,
}

struct RunnerGame {
    initialized: bool,
    player_parts: Vec<Entity>,
    player_offsets: Vec<Vec3>,
    cacti: Vec<CactusInstance>,
    score: f32,
    best_score: i32,
    speed: f32,
    base_speed: f32,
    jump_velocity: f32,
    gravity: f32,
    player_velocity_y: f32,
    player_anchor: Vec3,
    phase: RunPhase,
    last_log_score: i32,
}

impl RunnerGame {
    fn new() -> Self {
        Self {
            initialized: false,
            player_parts: Vec::new(),
            player_offsets: Vec::new(),
            cacti: Vec::new(),
            score: 0.0,
            best_score: 0,
            speed: 7.0,
            base_speed: 7.0,
            jump_velocity: 8.0,
            gravity: 24.0,
            player_velocity_y: 0.0,
            player_anchor: Vec3::new(-7.5, 0.6, 0.0),
            phase: RunPhase::Running,
            last_log_score: -1,
        }
    }
}

fn spawn_voxel(
    world: &mut World,
    mesh_index: usize,
    material_index: usize,
    translation: Vec3,
    scale: Vec3,
) -> Entity {
    let e = world.spawn();
    world.add_component(
        e,
        Transform {
            translation,
            rotation: Quat::IDENTITY,
            scale,
        },
    );
    world.add_component(
        e,
        Renderable {
            mesh_index,
            material_index,
        },
    );
    e
}

fn build_cactus(
    world: &mut World,
    mesh_index: usize,
    cactus_mat: usize,
    base_position: Vec3,
    variant: usize,
) -> CactusInstance {
    let mut parts = Vec::new();
    let mut local_offsets = Vec::new();

    let stem_h = 1.2 + variant as f32 * 0.35;
    for i in 0..(3 + variant) {
        let y = i as f32 * 0.7;
        let local = Vec3::new(0.0, y, 0.0);
        parts.push(spawn_voxel(
            world,
            mesh_index,
            cactus_mat,
            base_position + local,
            Vec3::new(0.7, 0.7, 0.7),
        ));
        local_offsets.push(local);
    }

    let arm_y = stem_h;
    let arm_len = 1.0 + variant as f32 * 0.2;
    for side in [-1.0_f32, 1.0_f32] {
        let local = Vec3::new(side * (0.55 + arm_len * 0.3), arm_y, 0.0);
        parts.push(spawn_voxel(
            world,
            mesh_index,
            cactus_mat,
            base_position + local,
            Vec3::new(0.7 * arm_len, 0.5, 0.7),
        ));
        local_offsets.push(local);
    }

    CactusInstance {
        parts,
        local_offsets,
        position: base_position,
        half_extents: Vec3::new(0.75 + variant as f32 * 0.2, 1.0 + variant as f32 * 0.25, 0.8),
    }
}

fn set_cactus_position(world: &mut World, cactus: &mut CactusInstance, position: Vec3) {
    cactus.position = position;
    for (idx, &part) in cactus.parts.iter().enumerate() {
        if let Some(t) = world.get_component_mut::<Transform>(part) {
            t.translation = position + cactus.local_offsets[idx];
        }
    }
}

fn set_player_pose(world: &mut World, game: &RunnerGame) {
    for (idx, &part) in game.player_parts.iter().enumerate() {
        if let Some(t) = world.get_component_mut::<Transform>(part) {
            t.translation = game.player_anchor + game.player_offsets[idx];
        }
    }
}

fn init_scene(world: &mut World, ctx: &mut RenderContext, game: &mut RunnerGame) {
    let cube_mesh = ctx.upload_mesh(&create_cube());
    let sand_mat = ctx.add_material(Material::solid(Vec3::new(0.83, 0.61, 0.36)));
    let dark_sand_mat = ctx.add_material(Material::solid(Vec3::new(0.62, 0.41, 0.27)));
    let canyon_mat = ctx.add_material(Material::solid(Vec3::new(0.55, 0.27, 0.22)));
    let sky_back_mat = ctx.add_material(Material::solid(Vec3::new(0.43, 0.24, 0.48)));
    let sky_top_mat = ctx.add_material(Material::solid(Vec3::new(0.92, 0.48, 0.30)));
    let dino_body_mat = ctx.add_material(Material::solid(Vec3::new(0.22, 0.81, 0.43)));
    let dino_eye_mat = ctx.add_material(Material::solid(Vec3::new(0.98, 0.98, 0.98)));
    let cactus_mat = ctx.add_material(Material::solid(Vec3::new(0.10, 0.52, 0.20)));

    let mut light = ctx.pbr_pass_light_uniform();
    light.ambient = [0.20, 0.13, 0.17, 1.0];
    light.dir_direction = [0.66, 0.92, 0.24, 0.0];
    light.dir_color = [1.75, 1.05, 0.70, 1.0];
    ctx.update_light_buffer(&light);

    // Sky backdrop blocks for sunset gradient illusion.
    spawn_voxel(
        world,
        cube_mesh,
        sky_back_mat,
        Vec3::new(10.0, 8.0, -22.0),
        Vec3::new(85.0, 26.0, 1.0),
    );
    spawn_voxel(
        world,
        cube_mesh,
        sky_top_mat,
        Vec3::new(10.0, 16.0, -21.8),
        Vec3::new(85.0, 8.0, 0.8),
    );

    // Runner lane.
    for i in -8..24 {
        let x = i as f32 * 3.2;
        spawn_voxel(
            world,
            cube_mesh,
            sand_mat,
            Vec3::new(x, -0.7, 0.0),
            Vec3::new(3.0, 1.0, 2.8),
        );
        spawn_voxel(
            world,
            cube_mesh,
            dark_sand_mat,
            Vec3::new(x + 1.3, -0.65, 0.95),
            Vec3::new(1.2, 0.9, 0.6),
        );
    }

    // Canyon walls and far silhouettes.
    for i in 0..18 {
        let x = -8.0 + i as f32 * 5.8;
        let h = 1.8 + (i % 4) as f32 * 1.1;
        spawn_voxel(
            world,
            cube_mesh,
            canyon_mat,
            Vec3::new(x, h * 0.5, -6.5),
            Vec3::new(3.0, h, 2.0),
        );
        spawn_voxel(
            world,
            cube_mesh,
            canyon_mat,
            Vec3::new(x + 1.5, h * 0.45, 5.2),
            Vec3::new(2.2, h * 0.9, 1.6),
        );
    }

    // Player (voxel dinosaur built from cubes around an anchor).
    let player_blueprint = [
        (Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.1, 1.0, 1.0), dino_body_mat), // body
        (Vec3::new(0.85, 0.35, 0.0), Vec3::new(0.8, 0.7, 0.8), dino_body_mat), // head
        (Vec3::new(-0.9, -0.1, 0.0), Vec3::new(0.8, 0.35, 0.7), dino_body_mat), // tail
        (Vec3::new(-0.25, -0.65, -0.2), Vec3::new(0.28, 0.55, 0.28), dino_body_mat), // leg
        (Vec3::new(0.25, -0.65, -0.2), Vec3::new(0.28, 0.55, 0.28), dino_body_mat),  // leg
        (Vec3::new(1.08, 0.50, 0.28), Vec3::new(0.14, 0.14, 0.14), dino_eye_mat),     // eye
    ];
    for (offset, scale, mat) in player_blueprint {
        game.player_offsets.push(offset);
        let e = spawn_voxel(world, cube_mesh, mat, game.player_anchor + offset, scale);
        game.player_parts.push(e);
    }

    // Cacti obstacles with different silhouettes.
    for i in 0..3 {
        let base = Vec3::new(14.0 + i as f32 * 10.0, -0.05, 0.0);
        game.cacti
            .push(build_cactus(world, cube_mesh, cactus_mat, base, i % 3));
    }

    game.initialized = true;
    log::info!("Dino Runner ready: Space/Up to jump, R to restart after crash.");
}

fn intersects_2d(a_pos: Vec3, a_scale: Vec3, b_pos: Vec3, b_scale: Vec3) -> bool {
    let a_half_x = a_scale.x * 0.5;
    let a_half_y = a_scale.y * 0.5;
    let b_half_x = b_scale.x * 0.5;
    let b_half_y = b_scale.y * 0.5;

    let overlap_x = (a_pos.x - b_pos.x).abs() <= (a_half_x + b_half_x);
    let overlap_y = (a_pos.y - b_pos.y).abs() <= (a_half_y + b_half_y);
    overlap_x && overlap_y
}

fn main() {
    let config = EngineConfig {
        window_width: 1280,
        window_height: 720,
        window_title: "RedOx Dino Runner - Voxel Sunset Canyon".to_string(),
        ..Default::default()
    };

    let mut game = RunnerGame::new();
    let mut last_update = Instant::now();
    let render_objects: Rc<RefCell<Vec<RenderObject>>> = Rc::new(RefCell::new(Vec::new()));
    let render_objects_for_prep = Rc::clone(&render_objects);
    let render_objects_for_render = Rc::clone(&render_objects);

    AppBuilder::new(config)
        .add_system(Stage::Update, move |world, ctx| {
            let now = Instant::now();
            let dt = now.duration_since(last_update).as_secs_f32().clamp(0.0, 0.05);
            last_update = now;

            if !game.initialized {
                init_scene(world, ctx, &mut game);
            }

            let mut jump_pressed = false;
            let mut restart_pressed = false;
            if let Some(input) = world.get_resource::<InputState>() {
                jump_pressed = input.keyboard.just_pressed(KeyCode::Space)
                    || input.keyboard.just_pressed(KeyCode::ArrowUp)
                    || input.keyboard.just_pressed(KeyCode::KeyW);
                restart_pressed = input.keyboard.just_pressed(KeyCode::KeyR);
            }

            if game.phase == RunPhase::GameOver && restart_pressed {
                game.phase = RunPhase::Running;
                game.score = 0.0;
                game.speed = game.base_speed;
                game.player_velocity_y = 0.0;
                game.player_anchor = Vec3::new(-7.5, 0.6, 0.0);
                set_player_pose(world, &game);
                for (i, cactus) in game.cacti.iter_mut().enumerate() {
                    let reset_pos = Vec3::new(14.0 + i as f32 * 10.0, -0.05, 0.0);
                    set_cactus_position(world, cactus, reset_pos);
                }
                log::info!("Run restarted.");
            }

            if game.phase == RunPhase::Running {
                let ground_y = 0.6;
                let grounded = game.player_anchor.y <= ground_y + 0.001;
                if grounded && jump_pressed {
                    game.player_velocity_y = game.jump_velocity;
                }
                game.player_velocity_y -= game.gravity * dt;
                game.player_anchor.y += game.player_velocity_y * dt;
                if game.player_anchor.y < ground_y {
                    game.player_anchor.y = ground_y;
                    game.player_velocity_y = 0.0;
                }
                set_player_pose(world, &game);

                let mut farthest_x = 6.0;
                for cactus in &game.cacti {
                    if cactus.position.x > farthest_x {
                        farthest_x = cactus.position.x;
                    }
                }

                let mut collided = false;
                for cactus in game.cacti.iter_mut() {
                    let mut x = cactus.position.x - game.speed * dt;
                    if x < -16.0 {
                        x = farthest_x + 8.0 + (game.score as i32 % 3) as f32;
                        farthest_x = x;
                    }
                    set_cactus_position(world, cactus, Vec3::new(x, cactus.position.y, 0.0));
                    if intersects_2d(
                        game.player_anchor,
                        Vec3::new(1.55, 1.35, 1.0),
                        cactus.position + Vec3::new(0.0, 0.55, 0.0),
                        cactus.half_extents,
                    ) {
                        collided = true;
                    }
                }

                game.score += dt * 10.0;
                game.speed = (game.base_speed + game.score * 0.016).min(13.5);

                if collided {
                    let final_score = game.score.floor() as i32;
                    if final_score > game.best_score {
                        game.best_score = final_score;
                    }
                    game.phase = RunPhase::GameOver;
                    log::info!(
                        "Game Over! score={}, best={}. Press R to restart.",
                        final_score,
                        game.best_score
                    );
                }
            }

            let whole_score = game.score.floor() as i32;
            if game.phase == RunPhase::Running
                && whole_score % 20 == 0
                && whole_score != game.last_log_score
                && whole_score > 0
            {
                game.last_log_score = whole_score;
                log::info!(
                    "Score: {}  Speed: {:.1}  Best: {}",
                    whole_score,
                    game.speed,
                    game.best_score
                );
            }

            // Side-view camera with slight tilt to show canyon depth.
            let camera = Camera::new(
                0.88,
                ctx.config.width as f32 / ctx.config.height as f32,
                0.1,
                200.0,
            );
            ctx.camera_uniform
                .update(&camera, Vec3::new(0.0, 6.4, 14.8), Quat::IDENTITY);
            ctx.update_camera_buffer();
        })
        .add_system(Stage::RenderPrep, move |world, _ctx| {
            let mut ro = render_objects_for_prep.borrow_mut();
            ro.clear();
            for entity in world.all_entities() {
                let (Some(transform), Some(renderable)) = (
                    world.get_component::<Transform>(entity),
                    world.get_component::<Renderable>(entity),
                ) else {
                    continue;
                };

                ro.push(RenderObject {
                    model_matrix: transform.matrix(),
                    color: [1.0, 1.0, 1.0, 1.0],
                    mesh_index: renderable.mesh_index,
                    material_index: renderable.material_index,
                });
            }
        })
        .add_system(Stage::Render, move |_world, ctx| {
            let ro = render_objects_for_render.borrow();
            if let Err(err) = ctx.render_frame(&ro) {
                match err {
                    wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated => {
                        ctx.resize(ctx.config.width, ctx.config.height);
                    }
                    wgpu::SurfaceError::OutOfMemory => {
                        log::error!("Out of GPU memory, exiting.");
                        std::process::exit(1);
                    }
                    wgpu::SurfaceError::Timeout => {
                        log::warn!("Surface timeout, skipping frame.");
                    }
                }
            }
        })
        .run();
}
