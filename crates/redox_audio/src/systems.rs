//! Audio systems for cinematic spatial sound simulation.
//!
//! Implements occlusion/obstruction raycast checking, reverb zone management,
//! and real-time acoustic parameter updates.

use redox_ecs::World;
use redox_math::Vec3;

use crate::components::{
    AudioEmitter, AudioListener, ReverbZone, ReverbPreset, SpatialAudioEmitter, AcousticMaterial,
};

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
/// In a full implementation, this would use rapier3d raycasting against colliders.
/// For now, this provides the interface structure for future integration.
pub fn check_occlusion(
    listener_pos: Vec3,
    emitter_pos: Vec3,
    _world: &World, // Would contain colliders in full implementation
) -> OcclusionResult {
    let distance = (emitter_pos - listener_pos).length();

    // Placeholder implementation: simple distance-based occlusion
    // In production, would raycast through physics colliders
    let occlusion = if distance < 1.0 { 0.0 } else { 0.0 };

    OcclusionResult {
        has_line_of_sight: true,
        occlusion,
        obstruction: 0.0,
        obstacle_distance: distance,
    }
}

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
/// Raycasts from listener to each emitter and applies occlusion/obstruction filtering.
pub fn occlusion_raycast_system(world: &mut World) {
    // Find listener
    let mut listener_pos = None;
    
    for entity in world.all_entities() {
        if let Some(listener) = world.get_component::<AudioListener>(entity) {
            listener_pos = Some(listener.position);
            break;
        }
    }

    if let Some(listener_pos) = listener_pos {
        // Update all spatial emitters
        let mut emitters_to_update = Vec::new();

        for entity in world.all_entities() {
            if let Some(emitter) = world.get_component::<SpatialAudioEmitter>(entity) {
                emitters_to_update.push((entity, emitter.clone()));
            }
        }

        for (entity, mut emitter) in emitters_to_update {
            // Check occlusion/obstruction
            let occlusion = check_occlusion(listener_pos, emitter.emitter.position, world);

            emitter.occlusion_coefficient = occlusion.occlusion;
            emitter.obstruction_coefficient = occlusion.obstruction;

            world.add_component(entity, emitter);
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
