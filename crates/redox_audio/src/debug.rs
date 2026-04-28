//! Debug visualization data for the audio system.
//!
//! When the [`AudioDebugDraw`] resource is present in the world, the occlusion system
//! (and optionally reverb system) can fill it with ray segments and zone bounds for
//! 3D visualization (e.g. listener→emitter rays, reverb zone boxes).

use redox_math::Vec3;

/// Resource for audio debug visualization.
///
/// Insert this into the world when you want to draw occlusion rays and/or reverb zones.
/// The [`crate::systems::occlusion_raycast_system`] will fill [`rays`](Self::rays) when
/// [`draw_rays`](Self::draw_rays) is true. A separate system or the game can fill
/// [`zone_boxes`](Self::zone_boxes) from reverb zone entities when [`draw_zones`](Self::draw_zones) is true.
#[derive(Debug, Default)]
pub struct AudioDebugDraw {
    /// When true, occlusion rays (listener → emitter) are written to [`rays`](Self::rays) each frame.
    pub draw_rays: bool,
    /// When true, reverb zone bounds can be written to [`zone_boxes`](Self::zone_boxes) (e.g. by the game).
    pub draw_zones: bool,
    /// Ray segments for 3D drawing: (origin, end, occluded).
    /// Green = not occluded, red = occluded.
    pub rays: Vec<(Vec3, Vec3, bool)>,
    /// Reverb zone axis-aligned boxes: (min, max) in world space.
    pub zone_boxes: Vec<(Vec3, Vec3)>,
}

impl AudioDebugDraw {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears rays and optionally zone_boxes. Call at the start of filling.
    pub fn clear(&mut self) {
        self.rays.clear();
        if self.draw_zones {
            self.zone_boxes.clear();
        }
    }
}
