//! Per-frame game simulation.

use super::*;

pub(crate) fn tick_game(
    world: &mut World,
    assets: &ArenaAssets,
    session: &mut Session,
    input: Option<&InputState>,
    dt: f32,
) {
    if input.is_some_and(|i| i.keyboard.just_pressed(KeyCode::KeyR)) {
        reset_run(world, assets, session);
        return;
    }

    if let Some(input) = input {
        if input.keyboard.is_pressed(KeyCode::KeyQ) {
            session.camera_yaw -= dt * 1.6;
        }
        if input.keyboard.is_pressed(KeyCode::KeyE) {
            session.camera_yaw += dt * 1.6;
        }
    }

    if session.phase != Phase::Playing {
        return;
    }

    session.time_played += dt;

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
    if move_dir.length_squared() > 0.0 {
        session.dash_dir = move_dir.normalize();
    }

    let dash_pressed = input.is_some_and(|i| {
        i.keyboard.just_pressed(KeyCode::ShiftLeft) || i.keyboard.just_pressed(KeyCode::ShiftRight)
    });
    let mut dashed = false;
    if session.dash_timer > 0.0 {
        session.dash_timer = (session.dash_timer - dt).max(0.0);
    } else if dash_pressed {
        let player = session.player;
        let dash_cost = world
            .get_component::<Ability>(player)
            .filter(|a| a.ready())
            .map(|a| (a.cost_stamina, a.cooldown));
        let has_stamina = world
            .get_component::<AttributeSet>(player)
            .map(|s| s.stamina.current);
        if let (Some((cost, cooldown)), Some(stamina)) = (dash_cost, has_stamina)
            && stamina >= cost
        {
            world
                .get_component_mut::<AttributeSet>(player)
                .expect("player attrs")
                .stamina
                .current -= cost;
            let ability = world
                .get_component_mut::<Ability>(player)
                .expect("player ability");
            ability.remaining_cooldown = cooldown;
            session.dash_timer = 0.22;
            dashed = true;
        }
    }
    if dashed {
        let audio = with_audio(world);
        let _ = audio.play_sfx("dash", 1.0);
        audio.mixer.bus_mut(MixerBusId::Sfx).set_volume(1.0);
        audio.sync_mixer();
    }

    let walk_dir = if move_dir.length_squared() > 0.0 {
        move_dir.normalize()
    } else {
        Vec3::ZERO
    };
    let dir = if session.dash_timer > 0.0 {
        session.dash_dir
    } else {
        walk_dir
    };
    let speed = if session.dash_timer > 0.0 { 16.0 } else { 5.5 };
    if dir.length_squared() > 0.0
        && let Some(t) = world.get_component_mut::<Transform>(session.player)
    {
        t.translation += dir * speed * dt;
        t.translation.x = t.translation.x.clamp(-half_bounds(), half_bounds());
        t.translation.z = t.translation.z.clamp(-half_bounds(), half_bounds());
        t.translation.y = if session.dash_timer > 0.0 {
            0.85 + (session.dash_timer * 12.0).sin().abs() * 0.15
        } else {
            0.85
        };
    }

    let player_pos = world
        .get_component::<Transform>(session.player)
        .map(|t| t.translation)
        .unwrap_or(Vec3::ZERO);

    if session.dash_timer > 0.0 {
        session.particles.spawn_from_emitter(
            &mut session.dash_emitter.emitter,
            dt,
            player_pos,
            -session.dash_dir,
        );
    }
    // GPU compute owns integration when the renderer particle system is live.
    session.stream_bytes = session.particles.buffer().upload_bytes().len();

    if let Some(attrs) = world.get_component_mut::<AttributeSet>(session.player) {
        attrs.stamina.current = (attrs.stamina.current + 18.0 * dt).min(attrs.stamina.max);
    }

    // World Partition streams around the player (camera-follow proxy).
    if let Some(report) = tick_streaming(world, player_pos) {
        session.last_stream = report;
    }

    // Guardrail + diagnostics: PCG once spawned props at ~1e8 m; those destroy
    // float precision and flash the whole HDR buffer white via bloom.
    {
        let mut max_r = 0.0_f32;
        let mut far_n = 0u32;
        let q = world.query::<(&StreamedActor, &Transform)>();
        for (actor, t) in q.iter(world) {
            let r = t.translation.length();
            max_r = max_r.max(r);
            if r > 500.0 {
                far_n += 1;
                if far_n == 1 {
                    log::warn!(
                        "streamed actor '{}' kind={} at ({:.1},{:.1},{:.1}) |r|={:.0}",
                        actor.id,
                        actor.kind,
                        t.translation.x,
                        t.translation.y,
                        t.translation.z,
                        r
                    );
                }
            }
        }
        if far_n > 0 && ((session.time_played * 2.0) as u32).is_multiple_of(4) {
            log::warn!(
                "streamed actors beyond 500m: {far_n} (max |r|={max_r:.0}) — regen worlds/arena"
            );
        }
    }

    for (entity, sentinel) in &mut session.sentinels {
        let s_pos = world
            .get_component::<Transform>(*entity)
            .map(|t| t.translation)
            .unwrap_or(Vec3::ZERO);
        let dist = (player_pos - s_pos).length();

        let state_name = if let Some(agent) = world.get_component_mut::<StateTreeComponent>(*entity)
        {
            agent.tick_with_distance(dist, dt);
            agent.tree.active_name().to_string()
        } else {
            "Patrol".to_string()
        };

        match state_name.as_str() {
            "Chase" => {
                let to_player = player_pos - s_pos;
                let flat = Vec3::new(to_player.x, 0.0, to_player.z);
                if flat.length_squared() > 1e-4 {
                    let step = flat.normalize() * 4.2 * dt;
                    if let Some(t) = world.get_component_mut::<Transform>(*entity) {
                        t.translation += step;
                        t.translation.y = 0.9;
                        t.rotation = Quat::from_rotation_y(flat.x.atan2(flat.z));
                    }
                }
            }
            _ => {
                // Patrol — orbital path around hub.
                sentinel.angle += sentinel.speed * dt;
                let offset = Vec3::new(sentinel.angle.cos(), 0.0, sentinel.angle.sin())
                    * sentinel.orbit_radius;
                if let Some(t) = world.get_component_mut::<Transform>(*entity) {
                    t.translation = sentinel.hub + offset + Vec3::new(0.0, 0.9, 0.0);
                    t.rotation =
                        Quat::from_rotation_y(sentinel.angle + std::f32::consts::FRAC_PI_2);
                }
            }
        }

        let s_pos = world
            .get_component::<Transform>(*entity)
            .map(|t| t.translation)
            .unwrap_or(Vec3::ZERO);
        if !GOD_MODE
            && (player_pos - s_pos).length() < 1.45
            && let Some(effects) = world.get_component_mut::<ActiveEffects>(session.player)
        {
            let burning = effects
                .effects
                .iter()
                .any(|e| e.duration > 0.0 && e.health_delta_per_sec < 0.0);
            if !burning {
                effects.effects.push(GameplayEffect::dot(14.0, 0.35));
            }
        }
    }

    for &crystal_e in &session.crystals {
        let spin = world.get_component_mut::<Crystal>(crystal_e).map(|c| {
            c.spin += dt * 1.4;
            c.spin
        });
        if let Some(spin) = spin
            && let Some(t) = world.get_component_mut::<Transform>(crystal_e)
        {
            t.rotation = Quat::from_rotation_y(spin);
        }
    }

    let mut picked = Vec::new();
    session.crystals.retain(|&e| {
        let pos = world
            .get_component::<Transform>(e)
            .map(|t| t.translation)
            .unwrap_or(Vec3::ZERO);
        if (player_pos - pos).length() < 1.35 {
            picked.push(pos);
            world.despawn(e);
            false
        } else {
            true
        }
    });
    for pos in picked {
        session.collected += 1;
        session.particles.spawn_from_emitter(
            &mut session.pickup_emitter.emitter,
            0.05,
            pos,
            Vec3::Y,
        );
        if let Some(attrs) = world.get_component_mut::<AttributeSet>(session.player) {
            attrs.health.current = (attrs.health.current + 12.0).min(attrs.health.max);
        }
        let audio = with_audio(world);
        let _ = audio.play_sfx("pickup", 0.85);
        audio
            .mixer
            .bus_mut(MixerBusId::Music)
            .set_volume(0.5 + session.collected as f32 * 0.05);
        audio.sync_mixer();
    }

    if GOD_MODE && let Some(attrs) = world.get_component_mut::<AttributeSet>(session.player) {
        attrs.health.current = attrs.health.max;
    }

    if session.collected >= CRYSTAL_COUNT as u8 {
        session.phase = Phase::Won;
        session.best_time = session.best_time.min(session.time_played);
    } else if !GOD_MODE
        && world
            .get_component::<AttributeSet>(session.player)
            .is_some_and(|a| a.health.current <= 0.0)
    {
        session.phase = Phase::Lost;
    }
}
