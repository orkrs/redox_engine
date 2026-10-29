//! ECS component describing one reflection probe and its capture settings.

use super::probe::ReflectionProbe;

/// Default capture resolution (texels per cube face).
pub const DEFAULT_PROBE_RESOLUTION: u32 = 128;

/// Smallest accepted capture resolution.
const MIN_RESOLUTION: u32 = 32;
/// Largest accepted capture resolution.
const MAX_RESOLUTION: u32 = 256;

/// Whether `resolution` is a power of two in `32..=256`.
pub fn is_valid_resolution(resolution: u32) -> bool {
    resolution.is_power_of_two() && (MIN_RESOLUTION..=MAX_RESOLUTION).contains(&resolution)
}

/// A reflection probe attached to an entity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReflectionProbeComponent {
    /// Influence box and priority.
    pub probe: ReflectionProbe,
    /// Capture resolution per cube face, a power of two in `32..=256`.
    pub resolution: u32,
    /// Set when the capture is out of date and should be recaptured.
    pub dirty: bool,
}

impl ReflectionProbeComponent {
    /// Creates a dirty component with the default resolution.
    pub fn new(probe: ReflectionProbe) -> Self {
        Self {
            probe,
            resolution: DEFAULT_PROBE_RESOLUTION,
            dirty: true,
        }
    }

    /// Returns the component with another capture resolution, or `None` if it is invalid.
    pub fn with_resolution(self, resolution: u32) -> Option<Self> {
        is_valid_resolution(resolution).then_some(Self { resolution, ..self })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_math::Vec3;

    fn probe() -> ReflectionProbe {
        ReflectionProbe {
            center: Vec3::ZERO,
            half_extent: Vec3::ONE,
            blend_distance: 0.1,
            priority: 0,
        }
    }

    #[test]
    fn resolution_must_be_a_power_of_two_between_32_and_256() {
        assert!(is_valid_resolution(32));
        assert!(is_valid_resolution(256));
        assert!(!is_valid_resolution(16));
        assert!(!is_valid_resolution(512));
        assert!(!is_valid_resolution(100));
    }

    #[test]
    fn a_new_component_is_dirty_at_the_default_resolution() {
        let component = ReflectionProbeComponent::new(probe());
        assert_eq!(component.resolution, DEFAULT_PROBE_RESOLUTION);
        assert!(component.dirty);
        assert!(component.with_resolution(64).is_some());
        assert!(component.with_resolution(48).is_none());
    }
}
