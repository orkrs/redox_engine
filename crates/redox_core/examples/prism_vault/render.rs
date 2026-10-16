//! Camera, lights and render-object extraction.

use super::*;

pub(crate) fn update_camera(
    world: &World,
    session: &Session,
    ctx: &RenderContext,
) -> (Vec3, Quat, Camera) {
    let target = world
        .get_component::<Transform>(session.player)
        .map(|t| t.translation + Vec3::new(0.0, 0.8, 0.0))
        .unwrap_or(Vec3::Y);
    let dist = 18.0;
    let pitch = 0.95_f32;
    let yaw = session.camera_yaw;
    let offset = Vec3::new(
        yaw.sin() * pitch.cos() * dist,
        pitch.sin() * dist + 4.0,
        yaw.cos() * pitch.cos() * dist,
    );
    let mut cam_pos = target + offset;
    let inner = ARENA * 0.5 - 2.0;
    cam_pos.x = cam_pos.x.clamp(-inner, inner);
    cam_pos.z = cam_pos.z.clamp(-inner, inner);
    let to_target = target - cam_pos;
    let cam_rot = if to_target.length_squared() > 1e-8 {
        Quat::from_rotation_arc(Vec3::NEG_Z, to_target.normalize())
    } else {
        Quat::IDENTITY
    };
    let camera = Camera::new(
        0.82,
        ctx.config.width as f32 / ctx.config.height as f32,
        0.1,
        250.0,
    );
    (cam_pos, cam_rot, camera)
}

pub(crate) fn build_lights(time: f32, vsm: bool) -> LightScene {
    let mut sun = DirectionalLight::new(
        Vec3::new(-0.45, 1.0, 0.25),
        Vec3::new(1.0, 0.94, 0.88),
        1.25,
    );
    // VSM-only when enabled: temporal SMRT screen pass (soft_shadows=2 in LightUniform).
    sun.vsm = vsm;

    let mut points = Vec::new();
    for i in 0..6 {
        let a = i as f32 / 6.0 * TAU + time * 0.08;
        let r = 11.0;
        let mut l = PointLight::new(
            Vec3::new(a.cos() * r, 3.8 + (a * 2.0).sin() * 0.4, a.sin() * r),
            match i % 3 {
                0 => Vec3::new(1.0, 0.35, 0.2),
                1 => Vec3::new(0.25, 0.5, 1.0),
                _ => Vec3::new(0.4, 1.0, 0.55),
            },
            14.0,
            9.0,
        );
        l.cast_shadows = true;
        points.push(l);
    }

    LightScene {
        directional: sun,
        ambient: Vec3::new(0.08, 0.09, 0.12),
        points,
        spots: vec![],
    }
}

pub(crate) fn extract_objects(
    world: &World,
    assets: &ArenaAssets,
    ctx: &RenderContext,
) -> Vec<RenderObject> {
    let mut out = Vec::new();
    for e in world.all_entities() {
        if let (Some(t), Some(r)) = (
            world.get_component::<Transform>(e),
            world.get_component::<Renderable>(e),
        ) {
            let bounds = ctx
                .get_mesh_bounds(r.mesh_index)
                .map(|b| b.transform(t.matrix()))
                .unwrap_or_else(|| redox_math::Aabb {
                    min: Vec3::splat(-1.0e9),
                    max: Vec3::splat(1.0e9),
                });
            out.push(RenderObject {
                model_matrix: t.matrix(),
                color: [1.0; 4],
                mesh_index: r.mesh_index,
                material_index: r.material_index,
                bounds,
            });
            continue;
        }

        // Streamed OFPA actors (props / walls) — decorative only.
        let Some(actor) = world.get_component::<StreamedActor>(e) else {
            continue;
        };
        let Some(t) = world.get_component::<Transform>(e) else {
            continue;
        };
        let (scale, mat) = match actor.kind.as_str() {
            "wall" => (Vec3::new(2.0, 3.0, 2.0), assets.stream_wall_mat),
            "prop" => (Vec3::new(0.6, 0.9, 0.6), assets.stream_prop_mat),
            _ => continue,
        };
        // Broken PCG offsets put props at 1e7+ m; skip them so MVP math cannot
        // NaN the HDR target when the camera faces that direction.
        if t.translation.length() > 500.0 {
            continue;
        }
        let model = Transform {
            translation: t.translation,
            rotation: t.rotation,
            scale,
        }
        .matrix();
        let bounds = ctx
            .get_mesh_bounds(assets.cube)
            .map(|b| b.transform(model))
            .unwrap_or_else(|| redox_math::Aabb {
                min: Vec3::splat(-1.0e9),
                max: Vec3::splat(1.0e9),
            });
        out.push(RenderObject {
            model_matrix: model,
            color: [1.0; 4],
            mesh_index: assets.cube,
            material_index: mat,
            bounds,
        });
    }
    out
}
