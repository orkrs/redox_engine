//! Audio systems for cinematic spatial sound simulation.
//!
//! Implements occlusion/obstruction raycast checking (via rapier3d when [`redox_physics::PhysicsContext`]
//! is present), reverb zone management, and real-time acoustic parameter updates.

use redox_ecs::World;
use redox_math::{GlobalTransform, Mat4, Transform, Vec3};

use crate::components::{AudioListener, ReverbPreset, ReverbZone, SpatialAudioEmitter};

/// Result of occlusion/obstruction raycast check.
#[derive(Debug, Clone, Copy)]
pub struct OcclusionResult {
    /// Whether there's a direct line of sight (unobstructed)
    pub has_line_of_sight: bool,
    /// Occlusion coefficient (0.0 = no occlusion, 1.0 = fully occluded)
    pub occlusion: f32,
    /// Obstruction coefficient (0.0 = no obstruction, 1.0 = fully obstructed)
    pub obstruction: f32,
    /// Distance to nearest obstacle
    pub obstacle_distance: f32,
}

impl Default for OcclusionResult {
    fn default() -> Self {
        Self {
            has_line_of_sight: true,
            occlusion: 0.0,
            obstruction: 0.0,
            obstacle_distance: f32::MAX,
        }
    }
}

/// Performs occlusion/obstruction raycast check between listener and emitter.
///
/// Used as fallback when [`redox_physics::PhysicsContext`] is not in the world (no physics).
/// When physics is available, [`occlusion_raycast_system`] uses real rapier3d raycasts instead.
pub fn check_occlusion(listener_pos: Vec3, emitter_pos: Vec3, _world: &World) -> OcclusionResult {
    let distance = (emitter_pos - listener_pos).length();
    OcclusionResult {
        has_line_of_sight: true,
        occlusion: 0.0,
        obstruction: 0.0,
        obstacle_distance: distance,
    }
}

/// Occlusion coefficient applied when a raycast hits an obstacle (one or more walls).
const OCCLUSION_COEFFICIENT_PER_HIT: f32 = 0.8;

/// System that tracks listener position relative to reverb zones.
///
/// Interpolates between zones for smooth transitions.
pub fn reverb_listener_system(world: &mut World) {
    // Find listener position
    let mut listener_pos = None;

    for entity in world.all_entities() {
        if let Some(listener) = world.get_component::<AudioListener>(entity) {
            listener_pos = Some(listener.position);
            break;
        }
    }

    let Some(listener_pos) = listener_pos else {
        return;
    };

    let mut zones_to_update = Vec::new();
    for entity in world.all_entities() {
        if let Some(zone) = world.get_component::<ReverbZone>(entity) {
            let pose = zone_pose(world, entity);
            zones_to_update.push((entity, zone.clone(), pose));
        }
    }

    for (entity, mut zone, pose) in zones_to_update {
        let distance = distance_to_zone(listener_pos, &pose, zone.half_extents);
        zone.listener_inside = distance <= 0.0;
        zone.current_weight = zone_weight(distance, zone.blend_distance);
        world.add_component(entity, zone);
    }
}

/// World matrix of a zone entity: `GlobalTransform`, else `Transform`, else identity.
fn zone_pose(world: &World, entity: redox_ecs::Entity) -> Mat4 {
    if let Some(global) = world.get_component::<GlobalTransform>(entity) {
        return global.matrix;
    }
    if let Some(t) = world.get_component::<Transform>(entity) {
        return Mat4::from_scale_rotation_translation(t.scale, t.rotation, t.translation);
    }
    Mat4::IDENTITY
}

/// Distance from `point` to the zone box (0.0 when inside).
///
/// The box is axis-aligned in the zone's local space; the distance is measured in local units.
pub fn distance_to_zone(point: Vec3, zone_matrix: &Mat4, half_extents: Vec3) -> f32 {
    let local = zone_matrix.inverse().transform_point3(point);
    let outside = (local.abs() - half_extents).max(Vec3::ZERO);
    outside.length()
}

/// Blend weight: 1.0 inside the zone, fading linearly to 0.0 over `blend_distance` outside.
pub fn zone_weight(distance: f32, blend_distance: f32) -> f32 {
    if distance <= 0.0 {
        return 1.0;
    }
    (1.0 - distance / blend_distance.max(f32::EPSILON)).clamp(0.0, 1.0)
}

/// System that updates occlusion coefficients for spatial audio emitters.
///
/// When [`redox_physics::PhysicsContext`] is present, performs real rapier3d raycasts from
/// listener to each emitter (excluding listener and emitter from the ray). Otherwise
/// falls back to [`check_occlusion`] (no occlusion). Also writes per-emitter occlusion
/// into [`crate::context::AudioContext`] via `AudioContext::set_emitter_occlusion` for
/// volume/low-pass application.
pub fn occlusion_raycast_system(world: &mut World) {
    let listener_entity = world
        .all_entities()
        .find(|&e| world.get_component::<AudioListener>(e).is_some());
    let listener_pos = listener_entity
        .and_then(|e| world.get_component::<AudioListener>(e))
        .map(|l| l.position);

    let Some(listener_pos) = listener_pos else {
        return;
    };

    let emitters: Vec<(redox_ecs::Entity, Vec3)> = world
        .all_entities()
        .filter_map(|e| {
            world
                .get_component::<SpatialAudioEmitter>(e)
                .map(|s| (e, s.emitter.position))
        })
        .collect();

    let use_physics = world
        .get_resource::<redox_physics::PhysicsContext>()
        .is_some();
    let mut results: Vec<(redox_ecs::Entity, Vec3, f32, f32)> = Vec::with_capacity(emitters.len());

    if use_physics {
        let physics = world
            .get_resource::<redox_physics::PhysicsContext>()
            .unwrap();
        for (entity, emitter_pos) in &emitters {
            let mut exclude = vec![*entity];
            if let Some(le) = listener_entity {
                exclude.push(le);
            }
            let (occlusion, obstruction) =
                match physics.cast_ray_excluding(listener_pos, *emitter_pos, &exclude) {
                    Some((_toi, _hit_entity)) => (OCCLUSION_COEFFICIENT_PER_HIT, 0.0),
                    None => (0.0, 0.0),
                };
            results.push((*entity, *emitter_pos, occlusion, obstruction));
        }
    } else {
        for (entity, emitter_pos) in &emitters {
            let result = check_occlusion(listener_pos, *emitter_pos, world);
            results.push((*entity, *emitter_pos, result.occlusion, result.obstruction));
        }
    }

    if let Some(debug) = world.get_resource_mut::<crate::debug::AudioDebugDraw>()
        && debug.draw_rays
    {
        debug.rays.clear();
        for (_entity, emitter_pos, occlusion, _) in &results {
            debug
                .rays
                .push((listener_pos, *emitter_pos, *occlusion > 0.0));
        }
    }

    for (entity, _pos, occlusion, obstruction) in &results {
        if let Some(emitter) = world.get_component_mut::<SpatialAudioEmitter>(*entity) {
            emitter.occlusion_coefficient = *occlusion;
            emitter.obstruction_coefficient = *obstruction;
        }
    }
    if let Some(audio_ctx) = world.get_resource_mut::<crate::context::AudioContext>() {
        let mut max_occ = 0.0_f32;
        for (entity, _pos, occlusion, _) in &results {
            audio_ctx.set_emitter_occlusion(*entity, *occlusion);
            max_occ = max_occ.max(*occlusion);
        }
        audio_ctx.apply_occlusion_lowpass(max_occ);
    }
}

/// Computes current reverb parameters by blending active zones.
///
/// Called each frame to update the global reverb effect in AudioContext.
pub fn compute_active_reverb(world: &World) -> Option<ReverbPreset> {
    let mut weighted: Vec<(ReverbPreset, f32)> = Vec::new();

    for entity in world.all_entities() {
        if let Some(zone) = world.get_component::<ReverbZone>(entity)
            && zone.current_weight > 0.001
        {
            weighted.push((
                ReverbPreset::from_name(&zone.preset_name),
                zone.current_weight,
            ));
        }
    }

    blend_presets(&ReverbPreset::outdoor(), &weighted)
}

/// Weighted average of the zone presets, faded in from `neutral` by the total coverage
/// (sum of weights, clamped to 1.0). Returns `None` when no zone contributes.
pub fn blend_presets(
    neutral: &ReverbPreset,
    zones: &[(ReverbPreset, f32)],
) -> Option<ReverbPreset> {
    let total_weight: f32 = zones.iter().map(|(_, w)| w).sum();
    if zones.is_empty() || total_weight <= f32::EPSILON {
        return None;
    }

    let avg = |field: fn(&ReverbPreset) -> f32| -> f32 {
        zones.iter().map(|(p, w)| field(p) * w).sum::<f32>() / total_weight
    };
    let coverage = total_weight.min(1.0);
    let mix = |base: f32, target: f32| base * (1.0 - coverage) + target * coverage;

    let mut blended = *neutral;
    blended.early_delay = mix(neutral.early_delay, avg(|p| p.early_delay));
    blended.decay_time = mix(neutral.decay_time, avg(|p| p.decay_time));
    blended.level = mix(neutral.level, avg(|p| p.level));
    blended.damping = mix(neutral.damping, avg(|p| p.damping));
    blended.diffusion = mix(neutral.diffusion, avg(|p| p.diffusion));
    blended.width = mix(neutral.width, avg(|p| p.width));
    Some(blended)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occlusion_result_default() {
        let result = OcclusionResult::default();
        assert!(result.has_line_of_sight);
        assert_eq!(result.occlusion, 0.0);
    }

    #[test]
    fn reverb_preset_blending() {
        let preset1 = ReverbPreset::cathedral();
        let preset2 = ReverbPreset::studio();

        let blended = blend_presets(&ReverbPreset::outdoor(), &[(preset1, 0.5), (preset2, 0.5)])
            .expect("two active zones");
        assert!(blended.decay_time > preset2.decay_time);
        assert!(blended.decay_time < preset1.decay_time);
    }

    #[test]
    fn blend_returns_none_without_active_zones() {
        assert!(blend_presets(&ReverbPreset::outdoor(), &[]).is_none());
    }

    #[test]
    fn partial_coverage_stays_between_neutral_and_zone() {
        let neutral = ReverbPreset::outdoor();
        let cathedral = ReverbPreset::cathedral();
        let blended = blend_presets(&neutral, &[(cathedral, 0.25)]).unwrap();
        assert!(blended.decay_time > neutral.decay_time);
        assert!(blended.decay_time < cathedral.decay_time);
    }

    #[test]
    fn distance_is_zero_inside_and_grows_outside() {
        let m = Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0));
        let half = Vec3::splat(2.0);
        assert_eq!(distance_to_zone(Vec3::new(11.0, 0.5, 0.0), &m, half), 0.0);
        let d = distance_to_zone(Vec3::new(15.0, 0.0, 0.0), &m, half);
        assert!((d - 3.0).abs() < 1e-5);
    }

    #[test]
    fn weight_fades_over_blend_distance() {
        assert_eq!(zone_weight(0.0, 2.0), 1.0);
        assert!((zone_weight(1.0, 2.0) - 0.5).abs() < 1e-6);
        assert_eq!(zone_weight(5.0, 2.0), 0.0);
    }
}
