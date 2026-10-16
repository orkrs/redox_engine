//! 3D Google Chrome dino — full RedOx stack.
//!
//! Plugins · transform hierarchy · Rapier KCC jump · trigger hit · PBR shadows · spatial SFX.
//!
//! Run: `cargo run -p redox_core --example dino_runner --release`
//! Space / W / Up — jump · R — restart after crash

use std::f32::consts::TAU;
use std::sync::Arc;

use kira::Frame;
use kira::sound::static_sound::{StaticSoundData, StaticSoundSettings};
use rapier3d::prelude::*;
use redox_audio::AudioContext;
use redox_core::config::EngineConfig;
use redox_core::dispatcher::Stage;
use redox_core::{AppBuilder, AudioPlugin, PhysicsPlugin, RenderPlugin, Time};
use redox_ecs::{Entity, EventReader, Events, World, set_parent};
use redox_input::InputState;
use redox_math::{GlobalTransform, Quat, Vec3};
use redox_physics::{
    CharacterController, Collider, Kinematic, PhysicsContext, RigidBody, Trigger, TriggerEnter,
};
use redox_render::FrameRenderObjects;
use redox_render::camera::Camera;
use redox_render::context::RenderContext;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene, PointLight};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::systems::{RenderObject, Transform};
use winit::keyboard::KeyCode;

const PLAYER_X: f32 = -7.5;
const STAND_Y: f32 = 0.72;
const JUMP_SPEED: f32 = 11.6;
const JUMP_GRAVITY: f32 = 28.0;

#[derive(Clone, Copy)]
struct Renderable {
    mesh_index: usize,
    material_index: usize,
}

struct VoxelPart;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Running,
    GameOver,
}

struct DinoGame {
    ready: bool,
    phase: Phase,
    player: Option<Entity>,
    legs: Vec<Entity>,
    cacti: Vec<Entity>,
    score: f32,
    best: i32,
    speed: f32,
    vy: f32,
    time: f32,
    cube: usize,
    cactus_mat: usize,
    trigger_reader: EventReader<TriggerEnter>,
    last_logged: i32,
    jump_buf: u8,
}

impl DinoGame {
    fn new() -> Self {
        Self {
            ready: false,
            phase: Phase::Running,
            player: None,
            legs: Vec::new(),
            cacti: Vec::new(),
            score: 0.0,
            best: 0,
            speed: 8.0,
            vy: 0.0,
            time: 0.0,
            cube: 0,
            cactus_mat: 0,
            trigger_reader: EventReader::new(),
            last_logged: -1,
            jump_buf: 0,
        }
    }
}

fn spawn_voxel(
    world: &mut World,
    mesh: usize,
    mat: usize,
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
            mesh_index: mesh,
            material_index: mat,
        },
    );
    world.add_component(e, VoxelPart);
    e
}

fn attach_part(
    world: &mut World,
    parent: Entity,
    mesh: usize,
    mat: usize,
    local: Vec3,
    scale: Vec3,
) -> Entity {
    let e = spawn_voxel(world, mesh, mat, local, scale);
    set_parent(world, e, parent);
    e
}

fn add_fixed_box(world: &mut World, entity: Entity, translation: Vec3, hx: f32, hy: f32, hz: f32) {
    let mut phys = world.remove_resource::<PhysicsContext>().expect("physics");
    let (bh, cols) = phys
        .add_rigid_body(
            entity,
            RigidBodyBuilder::fixed().translation(redox_math::Vec3::new(
                translation.x,
                translation.y,
                translation.z,
            )),
            vec![ColliderBuilder::cuboid(hx, hy, hz)],
        )
        .expect("fixed body");
    world.insert_resource(phys);
    world.add_component(entity, RigidBody { handle: bh });
    world.add_component(entity, Collider { handle: cols[0] });
}

fn tone_beep(freq_hz: f32, duration_secs: f32, volume: f32) -> StaticSoundData {
    let sample_rate = 22_050u32;
    let n = (duration_secs * sample_rate as f32).max(1.0) as usize;
    let frames: Vec<Frame> = (0..n)
        .map(|i| {
            let t = i as f32 / sample_rate as f32;
            let env = (1.0 - t / duration_secs).clamp(0.0, 1.0);
            let s = (t * freq_hz * TAU).sin() * volume * env;
            Frame::from_mono(s)
        })
        .collect();
    StaticSoundData {
        sample_rate,
        frames: Arc::from(frames),
        settings: StaticSoundSettings::default(),
        slice: None,
    }
}

fn init_scene(world: &mut World, ctx: &mut RenderContext, game: &mut DinoGame) {
    ctx.set_aa_sharp(true);
    ctx.set_rcas_sharpness(0.28);
    ctx.set_ssao_enabled(true);
    if let Some(audio) = world.get_resource_mut::<AudioContext>() {
        audio.register_sfx_data("jump", tone_beep(620.0, 0.08, 0.32));
        audio.register_sfx_data("hit", tone_beep(140.0, 0.22, 0.45));
        audio.sync_mixer();
    }

    let cube = ctx.upload_mesh(&create_cube());
    game.cube = cube;
    let sand = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.83, 0.61, 0.36));
        m.roughness = 0.85;
        m
    });
    let dark = ctx.add_material(Material::solid(Vec3::new(0.62, 0.41, 0.27)));
    let canyon = ctx.add_material(Material::solid(Vec3::new(0.55, 0.27, 0.22)));
    let sky_back = ctx.add_material(Material::solid(Vec3::new(0.43, 0.24, 0.48)));
    let sky_top = ctx.add_material(Material::solid(Vec3::new(0.92, 0.48, 0.30)));
    let dino = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.22, 0.81, 0.43));
        m.roughness = 0.45;
        m
    });
    let eye = ctx.add_material(Material::solid(Vec3::new(0.98, 0.98, 0.98)));
    let cactus_mat = ctx.add_material(Material::solid(Vec3::new(0.10, 0.52, 0.20)));
    game.cactus_mat = cactus_mat;

    spawn_voxel(
        world,
        cube,
        sky_back,
        Vec3::new(10.0, 8.0, -22.0),
        Vec3::new(85.0, 26.0, 1.0),
    );
    spawn_voxel(
        world,
        cube,
        sky_top,
        Vec3::new(10.0, 16.0, -21.8),
        Vec3::new(85.0, 8.0, 0.8),
    );

    let ground = spawn_voxel(
        world,
        cube,
        sand,
        Vec3::new(18.0, -0.5, 0.0),
        Vec3::new(64.0, 1.0, 3.2),
    );
    add_fixed_box(world, ground, Vec3::new(18.0, -0.5, 0.0), 32.0, 0.5, 1.6);
    spawn_voxel(
        world,
        cube,
        dark,
        Vec3::new(18.0, -0.42, 1.4),
        Vec3::new(64.0, 0.85, 0.55),
    );

    for i in 0..14 {
        let x = -6.0 + i as f32 * 6.0;
        let h = 1.8 + (i % 4) as f32 * 1.1;
        spawn_voxel(
            world,
            cube,
            canyon,
            Vec3::new(x, h * 0.5, -6.5),
            Vec3::new(3.0, h, 2.0),
        );
        spawn_voxel(
            world,
            cube,
            canyon,
            Vec3::new(x + 1.5, h * 0.45, 5.2),
            Vec3::new(2.2, h * 0.9, 1.6),
        );
    }

    let player_y = STAND_Y;
    let player = world.spawn();
    world.add_component(
        player,
        Transform::from_translation(Vec3::new(PLAYER_X, player_y, 0.0)),
    );
    world.add_component(player, Kinematic);
    world.add_component(
        player,
        CharacterController {
            desired_translation: Vec3::ZERO,
            gravity: 0.0,
            grounded: false,
        },
    );

    {
        let mut phys = world.remove_resource::<PhysicsContext>().expect("physics");
        let (bh, cols) = phys
            .add_rigid_body(
                player,
                RigidBodyBuilder::kinematic_position_based()
                    .translation(redox_math::Vec3::new(PLAYER_X, player_y, 0.0)),
                vec![
                    ColliderBuilder::capsule_y(0.35, 0.32)
                        .active_collision_types(ActiveCollisionTypes::all())
                        .active_events(ActiveEvents::COLLISION_EVENTS),
                ],
            )
            .expect("player body");
        world.insert_resource(phys);
        world.add_component(player, RigidBody { handle: bh });
        world.add_component(player, Collider { handle: cols[0] });
    }

    attach_part(
        world,
        player,
        cube,
        dino,
        Vec3::ZERO,
        Vec3::new(1.1, 1.0, 1.0),
    );
    attach_part(
        world,
        player,
        cube,
        dino,
        Vec3::new(0.85, 0.35, 0.0),
        Vec3::new(0.8, 0.7, 0.8),
    );
    attach_part(
        world,
        player,
        cube,
        dino,
        Vec3::new(-0.9, -0.1, 0.0),
        Vec3::new(0.8, 0.35, 0.7),
    );
    let leg_l = attach_part(
        world,
        player,
        cube,
        dino,
        Vec3::new(-0.25, -0.65, -0.2),
        Vec3::new(0.28, 0.55, 0.28),
    );
    let leg_r = attach_part(
        world,
        player,
        cube,
        dino,
        Vec3::new(0.25, -0.65, -0.2),
        Vec3::new(0.28, 0.55, 0.28),
    );
    attach_part(
        world,
        player,
        cube,
        eye,
        Vec3::new(1.08, 0.50, 0.28),
        Vec3::new(0.14, 0.14, 0.14),
    );
    game.legs = vec![leg_l, leg_r];
    game.player = Some(player);

    for i in 0..3 {
        let x = 16.0 + i as f32 * 11.0;
        game.cacti.push(spawn_cactus(world, cube, cactus_mat, x, i));
    }

    game.ready = true;
    log::info!("Dino Runner (KCC + hierarchy). Space to jump, R to restart.");
}

fn spawn_cactus(world: &mut World, mesh: usize, mat: usize, x: f32, variant: usize) -> Entity {
    let root = world.spawn();
    let y = 0.15;
    world.add_component(root, Transform::from_translation(Vec3::new(x, y, 0.0)));
    world.add_component(root, Kinematic);
    world.add_component(root, Trigger);

    let stem_n = 3 + variant;
    for i in 0..stem_n {
        attach_part(
            world,
            root,
            mesh,
            mat,
            Vec3::new(0.0, i as f32 * 0.55, 0.0),
            Vec3::splat(0.7),
        );
    }
    let arm_y = 0.55 * (stem_n as f32 - 1.0);
    for side in [-1.0_f32, 1.0] {
        attach_part(
            world,
            root,
            mesh,
            mat,
            Vec3::new(side * 0.85, arm_y, 0.0),
            Vec3::new(0.9, 0.5, 0.7),
        );
    }

    let hy = 0.45 + variant as f32 * 0.22;
    {
        let mut phys = world.remove_resource::<PhysicsContext>().expect("physics");
        let (bh, col) = phys
            .add_trigger(
                root,
                Vec3::new(x, y, 0.0),
                ColliderBuilder::cuboid(0.4, hy, 0.35)
                    .translation(redox_math::Vec3::new(0.0, hy, 0.0)),
            )
            .expect("cactus trigger");
        world.insert_resource(phys);
        world.add_component(root, RigidBody { handle: bh });
        world.add_component(root, Collider { handle: col });
    }
    root
}

fn aabb_overlap(a: Vec3, a_half: Vec3, b: Vec3, b_half: Vec3) -> bool {
    (a.x - b.x).abs() <= a_half.x + b_half.x
        && (a.y - b.y).abs() <= a_half.y + b_half.y
        && (a.z - b.z).abs() <= a_half.z + b_half.z
}

fn cactus_hit(world: &World, player: Entity, cacti: &[Entity]) -> bool {
    let Some(pt) = world.get_component::<Transform>(player) else {
        return false;
    };
    let player_center = pt.translation;
    let player_half = Vec3::new(0.7, 0.62, 0.45);
    for &cactus in cacti {
        let Some(ct) = world.get_component::<Transform>(cactus) else {
            continue;
        };
        let cactus_center = ct.translation + Vec3::new(0.0, 0.85, 0.0);
        let cactus_half = Vec3::new(0.5, 0.85, 0.4);
        if aabb_overlap(player_center, player_half, cactus_center, cactus_half) {
            return true;
        }
    }
    false
}
fn recycle_cactus(world: &mut World, cactus: Entity, new_x: f32) {
    if let Some(t) = world.get_component_mut::<Transform>(cactus) {
        t.translation.x = new_x;
        t.translation.z = 0.0;
    }
}

fn lights() -> LightScene {
    let mut sun = DirectionalLight::new(
        Vec3::new(0.55, 0.82, 0.18),
        Vec3::new(1.75, 1.05, 0.70),
        1.55,
    );
    sun.soft_shadows = false;
    let mut lamp = PointLight::new(
        Vec3::new(PLAYER_X + 2.0, 3.2, 1.5),
        Vec3::new(1.0, 0.72, 0.4),
        18.0,
        14.0,
    );
    lamp.cast_shadows = true;
    LightScene {
        directional: sun,
        ambient: Vec3::new(0.16, 0.11, 0.14),
        points: vec![lamp],
        spots: vec![],
    }
}

fn extract_objects(world: &World, ctx: &RenderContext) -> Vec<RenderObject> {
    let ids: Vec<Entity> = world.all_entities().collect();
    let mut out = Vec::new();
    for e in ids {
        let Some(r) = world.get_component::<Renderable>(e).copied() else {
            continue;
        };
        let matrix = world
            .get_component::<GlobalTransform>(e)
            .map(|g| g.matrix)
            .or_else(|| world.get_component::<Transform>(e).map(|t| t.matrix()))
            .unwrap_or(redox_math::Mat4::IDENTITY);
        let bounds = ctx
            .get_mesh_bounds(r.mesh_index)
            .map(|b| b.transform(matrix))
            .unwrap_or_else(redox_math::Aabb::empty);
        out.push(RenderObject {
            model_matrix: matrix,
            color: [1.0; 4],
            mesh_index: r.mesh_index,
            material_index: r.material_index,
            bounds,
        });
    }
    out
}

fn handle_surface_err(ctx: &mut RenderContext, err: wgpu::SurfaceError) {
    match err {
        wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated => {
            let w = ctx.config.width;
            let h = ctx.config.height;
            ctx.resize(w, h);
        }
        wgpu::SurfaceError::OutOfMemory => {
            log::error!("Out of GPU memory, exiting.");
            std::process::exit(1);
        }
        wgpu::SurfaceError::Timeout => log::warn!("Surface timeout, skipping frame."),
    }
}

fn main() {
    let config = EngineConfig {
        window_width: 1280,
        window_height: 720,
        window_title: "RedOx — 3D Chrome Dino".to_string(),
        render_scale: 1.0,
        aa_sharp: true,
        ..Default::default()
    };

    AppBuilder::new(config)
        .add_plugin(PhysicsPlugin {
            gravity: Vec3::new(0.0, -9.81, 0.0),
        })
        .add_plugin(AudioPlugin)
        .add_plugin(RenderPlugin)
        .add_system(Stage::Update, |world, ctx| {
            let dt = world
                .get_resource::<Time>()
                .map(|t| t.delta_time)
                .unwrap_or(1.0 / 60.0);

            if world.get_resource::<DinoGame>().is_none() {
                world.insert_resource(DinoGame::new());
            }

            let jump;
            let restart;
            {
                let input = world.get_resource::<InputState>();
                jump = input.is_some_and(|i| {
                    i.keyboard.just_pressed(KeyCode::Space)
                        || i.keyboard.just_pressed(KeyCode::ArrowUp)
                        || i.keyboard.just_pressed(KeyCode::KeyW)
                });
                restart = input.is_some_and(|i| i.keyboard.just_pressed(KeyCode::KeyR));
            }

            let mut game = world.remove_resource::<DinoGame>().unwrap();
            if !game.ready {
                init_scene(world, ctx, &mut game);
            }

            let hits: Vec<TriggerEnter> = world
                .get_resource::<Events<TriggerEnter>>()
                .map(|ev| game.trigger_reader.iter(ev).copied().collect())
                .unwrap_or_default();

            if game.phase == Phase::GameOver && restart {
                game.phase = Phase::Running;
                game.score = 0.0;
                game.speed = 8.0;
                game.vy = 0.0;
                game.jump_buf = 0;
                if let Some(player) = game.player
                    && let Some(t) = world.get_component_mut::<Transform>(player)
                {
                    t.translation = Vec3::new(PLAYER_X, STAND_Y, 0.0);
                }
                for (i, &cactus) in game.cacti.iter().enumerate() {
                    recycle_cactus(world, cactus, 16.0 + i as f32 * 11.0);
                }
                log::info!("Run restarted.");
            }

            if game.phase == Phase::Running {
                game.time += dt;
                let player = game.player.expect("player");
                let y = world
                    .get_component::<Transform>(player)
                    .map(|t| t.translation.y)
                    .unwrap_or(STAND_Y);
                let grounded = y <= STAND_Y + 0.12;

                if jump {
                    game.jump_buf = 10;
                }
                if grounded && game.jump_buf > 0 {
                    game.vy = JUMP_SPEED;
                    game.jump_buf = 0;
                    if let Some(audio) = world.get_resource_mut::<AudioContext>() {
                        let _ = audio.play_sfx("jump", 1.0);
                    }
                }
                game.jump_buf = game.jump_buf.saturating_sub(1);
                game.vy -= JUMP_GRAVITY * dt;

                if let Some(t) = world.get_component_mut::<Transform>(player) {
                    t.translation.x = PLAYER_X;
                    t.translation.z = 0.0;
                    t.translation.y += game.vy * dt;
                    if t.translation.y <= STAND_Y {
                        t.translation.y = STAND_Y;
                        game.vy = 0.0;
                    }
                }
                if let Some(cc) = world.get_component_mut::<CharacterController>(player) {
                    cc.desired_translation = Vec3::ZERO;
                    cc.gravity = 0.0;
                }

                let bob = if grounded {
                    (game.time * 14.0).sin() * 0.08
                } else {
                    0.0
                };
                for (i, &leg) in game.legs.iter().enumerate() {
                    let side = if i == 0 { 1.0 } else { -1.0 };
                    if let Some(t) = world.get_component_mut::<Transform>(leg) {
                        t.translation.y = -0.65 + bob * side;
                    }
                }

                let mut farthest = 8.0_f32;
                for &c in &game.cacti {
                    if let Some(t) = world.get_component::<Transform>(c) {
                        farthest = farthest.max(t.translation.x);
                    }
                }
                let speed = game.speed;
                let score = game.score;
                let mut cacti = game.cacti.clone();
                for cactus in &mut cacti {
                    let mut x = world
                        .get_component::<Transform>(*cactus)
                        .map(|t| t.translation.x)
                        .unwrap_or(0.0)
                        - speed * dt;
                    if x < -16.0 {
                        x = farthest + 9.0 + (score as i32 % 4) as f32;
                        farthest = x;
                    }
                    recycle_cactus(world, *cactus, x);
                }
                game.cacti = cacti;

                game.score += dt * 10.0;
                game.speed = (8.0 + game.score * 0.018).min(16.0);

                let hit = cactus_hit(world, player, &game.cacti)
                    || hits.iter().any(|h| h.other == player || h.sensor == player);
                if hit {
                    let final_score = game.score.floor() as i32;
                    game.best = game.best.max(final_score);
                    game.phase = Phase::GameOver;
                    if let Some(audio) = world.get_resource_mut::<AudioContext>() {
                        let _ = audio.play_sfx("hit", 1.0);
                    }
                    log::info!(
                        "Game Over! score={final_score} best={}. Press R.",
                        game.best
                    );
                }
            }

            let whole = game.score.floor() as i32;
            if game.phase == Phase::Running
                && whole > 0
                && whole % 50 == 0
                && whole != game.last_logged
            {
                game.last_logged = whole;
                log::info!("score={whole} speed={:.1} best={}", game.speed, game.best);
            }

            let player_y = game
                .player
                .and_then(|p| world.get_component::<Transform>(p).map(|t| t.translation.y))
                .unwrap_or(1.0);
            world.insert_resource(game);

            let camera = Camera::new(
                0.88,
                ctx.config.width as f32 / ctx.config.height as f32,
                0.1,
                200.0,
            );
            let cam_pos = Vec3::new(PLAYER_X + 3.5, 6.2, 14.2);
            let look = Vec3::new(PLAYER_X + 2.0, player_y.max(0.6) + 0.4, 0.0) - cam_pos;
            let cam_rot = if look.length_squared() > 1e-8 {
                Quat::from_rotation_arc(Vec3::NEG_Z, look.normalize())
            } else {
                Quat::IDENTITY
            };
            ctx.camera_uniform.update(&camera, cam_pos, cam_rot);
            ctx.update_camera_buffer();
            world.insert_resource(camera);
        })
        .add_system(Stage::RenderPrep, |world, ctx| {
            let list = extract_objects(world, ctx);
            world
                .get_resource_mut::<FrameRenderObjects>()
                .expect("FrameRenderObjects")
                .0 = list;
        })
        .add_system(Stage::Render, |world, ctx| {
            let objects = {
                let frame = world
                    .get_resource_mut::<FrameRenderObjects>()
                    .expect("FrameRenderObjects");
                std::mem::take(&mut frame.0)
            };
            let camera = world
                .get_resource::<Camera>()
                .cloned()
                .unwrap_or_else(|| Camera::new(0.88, 16.0 / 9.0, 0.1, 200.0));
            let scene = lights();
            let fv = FrameView::new(&camera, &objects, &scene);
            match ctx.surface().get_current_texture() {
                Ok(output) => {
                    let view = output
                        .texture
                        .create_view(&wgpu::TextureViewDescriptor::default());
                    if let Err(err) = ctx.render_frame_view(&fv, &view) {
                        handle_surface_err(ctx, err);
                    }
                    output.present();
                }
                Err(err) => handle_surface_err(ctx, err),
            }
        })
        .run_with_setup(|world| {
            world.insert_resource(FrameRenderObjects::default());
        });
}
