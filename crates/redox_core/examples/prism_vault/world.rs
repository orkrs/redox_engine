//! Arena world: partition, procedural audio, arena build, run setup and reset.

use super::*;

pub(crate) fn arena_world_path() -> PathBuf {
    let from_manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../worlds/arena");
    if from_manifest.join("world.ron").exists() {
        return from_manifest;
    }
    PathBuf::from("worlds/arena")
}

pub(crate) fn open_or_generate_partition() -> WorldPartition {
    let path = arena_world_path();
    let params = PcgParams {
        name: "arena".into(),
        kind: PcgKind::Arena,
        width: 6,
        height: 6,
        cell_size: 16.0,
        seed: 42,
    };
    if !path.join("world.ron").exists()
        && let Err(e) = generate_to(&path, &params)
    {
        log::warn!("PCG write failed ({e}); falling back to in-memory partition");
        let generated = redox_scene::generate(&params);
        let mut part = WorldPartition::in_memory(
            params.cell_size,
            [0.0; 3],
            generated.cells.into_iter().collect(),
        );
        configure_streaming(&mut part);
        return part;
    }
    let mut part = WorldPartition::open(&path).unwrap_or_else(|e| {
        log::warn!("WorldPartition::open failed ({e}); using in-memory");
        let generated = redox_scene::generate(&params);
        WorldPartition::in_memory(
            params.cell_size,
            [0.0; 3],
            generated.cells.into_iter().collect(),
        )
    });
    configure_streaming(&mut part);
    part
}

pub(crate) fn configure_streaming(part: &mut WorldPartition) {
    part.config.load_radius = 1;
    part.config.unload_radius = 2;
    part.config.budget.max_loads = 2;
    part.config.budget.max_unloads = 4;
}

pub(crate) fn register_procedural_sfx(audio: &mut AudioContext) {
    audio.register_sfx_data("dash", tone_beep(420.0, 0.09, 0.35));
    audio.register_sfx_data("pickup", tone_beep(880.0, 0.12, 0.28));
}

pub(crate) fn tone_beep(freq_hz: f32, duration_secs: f32, volume: f32) -> StaticSoundData {
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
    }
}

pub(crate) fn sync_hud_stats(session: &mut Session, world: &World) {
    if let Some(a) = world.get_component::<AttributeSet>(session.player) {
        session.health = a.health.current;
        session.stamina = a.stamina.current;
    }
}

pub(crate) fn build_arena(world: &mut World, ctx: &mut RenderContext) -> ArenaAssets {
    let cube = ctx.upload_mesh(&create_cube());
    let sphere = ctx.upload_mesh(&create_sphere(0.5, 64));

    let floor_mat = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.18, 0.17, 0.20));
        m.roughness = 1.0;
        m
    });
    let wall_mat = ctx.add_material(Material::solid(Vec3::new(0.12, 0.13, 0.18)));
    let pillar_mat = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.35, 0.32, 0.38));
        m.metallic = 0.4;
        m.roughness = 0.35;
        m
    });
    let platform_mat = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.28, 0.26, 0.32));
        m.metallic = 0.6;
        m.roughness = 0.25;
        m
    });
    let sentinel_mat = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.9, 0.15, 0.12));
        m.emissive = Vec3::new(0.6, 0.05, 0.05);
        m.metallic = 0.7;
        m.roughness = 0.2;
        m
    });
    let player_mat = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.15, 0.55, 0.95));
        m.emissive = Vec3::new(0.05, 0.15, 0.35);
        m.metallic = 0.85;
        m.roughness = 0.18;
        m
    });
    let crystal_mat = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.25, 0.75, 1.0));
        m.emissive = Vec3::new(0.5, 1.4, 2.0);
        m.metallic = 0.95;
        m.roughness = 0.08;
        m
    });
    let stream_prop_mat = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.45, 0.4, 0.32));
        m.roughness = 0.7;
        m
    });
    let stream_wall_mat = ctx.add_material({
        let mut m = Material::solid(Vec3::new(0.18, 0.2, 0.26));
        m.metallic = 0.15;
        m.roughness = 0.8;
        m
    });

    spawn_mesh(
        world,
        cube,
        floor_mat,
        Vec3::ZERO,
        Vec3::new(ARENA, 0.35, ARENA),
    );
    let half = ARENA * 0.5;
    // Low curbs, not 5 m slabs: a 34×5 wall fills the FOV whenever the orbit
    // camera looks at it and bloom turns that face into a "glowing roof".
    for &(x, z) in &[(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
        spawn_mesh(
            world,
            cube,
            wall_mat,
            Vec3::new(x * half, 0.45, z * half),
            Vec3::new(
                if x == 0.0 { ARENA } else { 0.45 },
                0.9,
                if z == 0.0 { ARENA } else { 0.45 },
            ),
        );
    }
    for (i, (px, pz)) in [(8.0, 8.0), (-9.0, 7.0), (10.0, -8.0), (-8.0, -10.0)]
        .into_iter()
        .enumerate()
    {
        spawn_mesh(
            world,
            cube,
            pillar_mat,
            Vec3::new(px, 2.2, pz),
            Vec3::new(1.4, 4.4, 1.4),
        );
        if i % 2 == 0 {
            spawn_mesh(
                world,
                cube,
                platform_mat,
                Vec3::new(px, 4.8, pz),
                Vec3::new(3.5, 0.35, 3.5),
            );
        }
    }

    ArenaAssets {
        cube,
        sphere,
        sentinel_mat,
        player_mat,
        crystal_mat,
        stream_prop_mat,
        stream_wall_mat,
    }
}

pub(crate) fn spawn_run(world: &mut World, assets: &ArenaAssets) -> Session {
    let player = world.spawn();
    world.add_component(
        player,
        Transform {
            translation: Vec3::new(0.0, 0.85, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::new(0.9, 1.6, 0.9),
        },
    );
    world.add_component(
        player,
        Renderable {
            mesh_index: assets.cube,
            material_index: assets.player_mat,
        },
    );
    world.add_component(player, PlayerMarker);
    world.add_component(player, AttributeSet::default_character());
    world.add_component(player, Ability::new(1, 0.9, 25.0));
    world.add_component(player, ActiveEffects::default());

    let spots: [Vec3; CRYSTAL_COUNT] = [
        Vec3::new(6.0, 1.2, 5.0),
        Vec3::new(-7.0, 1.2, 4.0),
        Vec3::new(5.0, 5.2, 8.0),
        Vec3::new(-9.0, 5.2, 7.0),
        Vec3::new(0.0, 1.2, -10.0),
        Vec3::new(11.0, 1.2, -6.0),
        Vec3::new(-11.0, 1.2, -8.0),
        Vec3::new(0.0, 1.2, 12.0),
    ];

    let mut crystals = Vec::new();
    for (i, pos) in spots.into_iter().enumerate() {
        let e = world.spawn();
        world.add_component(
            e,
            Transform {
                translation: pos,
                rotation: Quat::from_rotation_y(i as f32 * 0.7),
                scale: Vec3::splat(1.0),
            },
        );
        // Regular mesh — VG meshlet cull (cone/Hi-Z) shreds small spheres into
        // glowing shards. Keep VG for large assets, not collectible orbs.
        world.add_component(
            e,
            Renderable {
                mesh_index: assets.sphere,
                material_index: assets.crystal_mat,
            },
        );
        world.add_component(
            e,
            Crystal {
                spin: i as f32 * 0.4,
            },
        );
        crystals.push(e);
    }

    // Hubs stay off spawn: an orbit through the origin insta-kills a standing player.
    let specs = [
        (Vec3::new(12.0, 0.9, 10.0), 5.5_f32, 0.0_f32, 0.55_f32),
        (Vec3::new(-11.0, 0.9, 9.0), 5.0, TAU * 0.25, 0.75),
        (Vec3::new(9.0, 0.9, -12.0), 5.0, TAU * 0.5, 0.65),
    ];
    let mut sentinels = Vec::new();
    for (hub, radius, angle, speed) in specs {
        let e = world.spawn();
        world.add_component(
            e,
            Transform {
                translation: hub + Vec3::new(radius, 0.0, 0.0),
                rotation: Quat::IDENTITY,
                scale: Vec3::new(1.1, 1.8, 1.1),
            },
        );
        world.add_component(
            e,
            Renderable {
                mesh_index: assets.cube,
                material_index: assets.sentinel_mat,
            },
        );
        world.add_component(e, StateTreeComponent::patrol_chase());
        sentinels.push((
            e,
            Sentinel {
                hub,
                orbit_radius: radius,
                angle,
                speed,
            },
        ));
    }

    let mut dash_emitter = ParticleEmitterComponent {
        emitter: ParticleEmitter {
            spawn_rate: 120.0,
            lifetime: 0.35,
            initial_speed: 4.0,
            color: Vec3::new(0.3, 0.7, 1.0),
            size: 0.08,
            ..Default::default()
        },
        world_offset: Vec3::ZERO,
    };
    dash_emitter.emitter.max_particles = 512;
    let mut pickup_emitter = ParticleEmitterComponent {
        emitter: ParticleEmitter {
            spawn_rate: 200.0,
            lifetime: 0.6,
            initial_speed: 6.0,
            color: Vec3::new(0.4, 1.0, 1.0),
            size: 0.1,
            ..Default::default()
        },
        world_offset: Vec3::ZERO,
    };
    pickup_emitter.emitter.max_particles = 512;

    Session {
        phase: Phase::Playing,
        player,
        crystals,
        sentinels,
        collected: 0,
        health: 100.0,
        stamina: 100.0,
        camera_yaw: 0.4,
        dash_timer: 0.0,
        dash_dir: Vec3::NEG_Z,
        particles: ParticleSystem::with_capacity(4096),
        dash_emitter,
        pickup_emitter,
        time_played: 0.0,
        best_time: f32::MAX,
        last_stream: StreamingReport::default(),
        stream_bytes: 0,
    }
}

pub(crate) fn reset_run(world: &mut World, assets: &ArenaAssets, session: &mut Session) {
    let best = session.best_time;
    world.despawn(session.player);
    for e in session.crystals.drain(..) {
        world.despawn(e);
    }
    for (e, _) in session.sentinels.drain(..) {
        world.despawn(e);
    }
    *session = spawn_run(world, assets);
    session.best_time = best;
    session.particles.buffer_mut().clear();
}

pub(crate) fn spawn_mesh(
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

pub(crate) fn half_bounds() -> f32 {
    ARENA * 0.5 - 1.5
}
