//! Audio systems for cinematic spatial sound simulation.
//!
//! Implements occlusion/obstruction raycast checking (via rapier3d when [`redox_physics::PhysicsContext`]
//! is present), reverb zone management, and real-time acoustic parameter updates.

use redox_ecs::World;
use redox_math::Vec3;

use crate::components::{AudioListener, ReverbZone, ReverbPreset, SpatialAudioEmitter};

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
pub fn check_occlusion(
    listener_pos: Vec3,
    emitter_pos: Vec3,
    _world: &World,
) -> OcclusionResult {
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

    if let Some(_listener_pos) = listener_pos {
        // Collect all reverb zones and update their listener status
        let mut zones_to_update = Vec::new();

        for entity in world.all_entities() {
            if let Some(zone) = world.get_component::<ReverbZone>(entity) {
                zones_to_update.push((entity, zone.clone()));
            }
        }

        // For each zone, check if listener is inside (would use collider bounds in full impl)
        for (entity, mut zone) in zones_to_update {
            // Placeholder: determine if listener is inside based on simple distance
            // In full implementation, would check against entity's Collider component
            let distance_to_zone = 0.0; // Would compute from collider bounds
            zone.listener_inside = distance_to_zone < 0.1;
            zone.current_weight = (1.0 - (distance_to_zone / zone.blend_distance).min(1.0)).max(0.0);

            world.add_component(entity, zone);
        }
    }
}

/// System that updates occlusion coefficients for spatial audio emitters.
///
/// When [`redox_physics::PhysicsContext`] is present, performs real rapier3d raycasts from
/// listener to each emitter (excluding listener and emitter from the ray). Otherwise
/// falls back to [`check_occlusion`] (no occlusion). Also writes per-emitter occlusion
/// into [`crate::context::AudioContext`] via [`AudioContext::set_emitter_occlusion`] for
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

    let use_physics = world.get_resource::<redox_physics::PhysicsContext>().is_some();
    let mut results: Vec<(redox_ecs::Entity, Vec3, f32, f32)> = Vec::with_capacity(emitters.len());

    if use_physics {
        let physics = world.get_resource::<redox_physics::PhysicsContext>().unwrap();
        for (entity, emitter_pos) in &emitters {
            let mut exclude = vec![*entity];
            if let Some(le) = listener_entity {
                exclude.push(le);
            }
            let (occlusion, obstruction) = match physics.cast_ray_excluding(
                listener_pos,
                *emitter_pos,
                &exclude,
            ) {
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

    if let Some(debug) = world.get_resource_mut::<crate::debug::AudioDebugDraw>() {
        if debug.draw_rays {
            debug.rays.clear();
            for (_entity, emitter_pos, occlusion, _) in &results {
                debug.rays.push((listener_pos, *emitter_pos, *occlusion > 0.0));
            }
        }
    }

    for (entity, _pos, occlusion, obstruction) in &results {
        if let Some(emitter) = world.get_component_mut::<SpatialAudioEmitter>(*entity) {
            emitter.occlusion_coefficient = *occlusion;
            emitter.obstruction_coefficient = *obstruction;
        }
    }
    if let Some(audio_ctx) = world.get_resource_mut::<crate::context::AudioContext>() {
        for (entity, _pos, occlusion, _) in &results {
            audio_ctx.set_emitter_occlusion(*entity, *occlusion);
        }
    }
}

/// Computes current reverb parameters by blending active zones.
///
/// Called each frame to update the global reverb effect in AudioContext.
pub fn compute_active_reverb(world: &World) -> Option<ReverbPreset> {
    let mut total_weight = 0.0;
    let mut blended = ReverbPreset::outdoor(); // Start with neutral

    // Collect all reverb zones
    let mut zones: Vec<ReverbPreset> = Vec::new();

    for entity in world.all_entities() {
        if let Some(zone) = world.get_component::<ReverbZone>(entity) {
            if zone.current_weight > 0.001 {
                let preset = ReverbPreset::from_name(&zone.preset_name);
                zones.push(preset);
                total_weight += zone.current_weight;
            }
        }
    }

    if zones.is_empty() {
        return None;
    }

    // Blend presets based on weights
    for preset in &zones {
        let weight = total_weight / (zones.len() as f32).max(1.0);

        blended.early_delay = blended.early_delay * (1.0 - weight) + preset.early_delay * weight;
        blended.decay_time = blended.decay_time * (1.0 - weight) + preset.decay_time * weight;
        blended.level = blended.level * (1.0 - weight) + preset.level * weight;
        blended.damping = blended.damping * (1.0 - weight) + preset.damping * weight;
        blended.diffusion = blended.diffusion * (1.0 - weight) + preset.diffusion * weight;
        blended.width = blended.width * (1.0 - weight) + preset.width * weight;
    }

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

        // Simulate blending 50/50
        let blended_decay = preset1.decay_time * 0.5 + preset2.decay_time * 0.5;
        assert!(blended_decay > preset2.decay_time);
        assert!(blended_decay < preset1.decay_time);
    }
}
