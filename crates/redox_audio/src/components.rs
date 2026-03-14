//! Audio ECS components.

use redox_asset::Handle;
use redox_math::Vec3;

use crate::asset_types::AudioData;

/// An audio source attached to an entity.
///
/// Uses a handle to [`AudioData`] loaded via the asset manager. When the asset
/// is loaded and synced to [`crate::context::AudioContext`], playback can use
/// [`AudioContext::play_sound_by_handle`] or [`AudioContext::play_spatial_by_handle`].
#[derive(Debug, Clone)]
pub struct AudioEmitter {
    /// Handle to the audio asset (WAV/OGG bytes). Playback uses the context's cached sound.
    pub sound_handle: Option<Handle<AudioData>>,
    /// World-space position of the source.
    pub position: Vec3,
    /// Volume multiplier (1.0 = full volume).
    pub volume: f32,
    /// Pitch multiplier (1.0 = normal pitch).
    pub pitch: f32,
    /// Whether the sound should loop.
    pub looping: bool,
    /// Whether the sound is currently playing.
    pub playing: bool,
    /// Maximum audible distance.
    pub max_distance: f32,
}

impl AudioEmitter {
    /// Creates a new emitter with no sound (sound_handle = None).
    pub fn new(position: Vec3) -> Self {
        Self {
            sound_handle: None,
            position,
            volume: 1.0,
            pitch: 1.0,
            looping: false,
            playing: false,
            max_distance: 50.0,
        }
    }

    /// Sets the sound to play via asset handle.
    pub fn with_sound(mut self, handle: Handle<AudioData>) -> Self {
        self.sound_handle = Some(handle);
        self
    }

    /// Convenience: starts playing.
    pub fn play(&mut self) {
        self.playing = true;
    }

    /// Convenience: stops playing.
    pub fn stop(&mut self) {
        self.playing = false;
    }
}

impl Default for AudioEmitter {
    fn default() -> Self {
        Self::new(Vec3::ZERO)
    }
}

/// The audio listener (usually attached to the camera entity).
///
/// There should be exactly one listener in the world.
#[derive(Debug, Clone)]
pub struct AudioListener {
    /// World-space position.
    pub position: Vec3,
    /// Forward direction (normalized).
    pub forward: Vec3,
    /// Up direction (normalized).
    pub up: Vec3,
}

impl AudioListener {
    pub fn new(position: Vec3, forward: Vec3, up: Vec3) -> Self {
        Self {
            position,
            forward: forward.normalize(),
            up: up.normalize(),
        }
    }
}

impl Default for AudioListener {
    fn default() -> Self {
        Self::new(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y)
    }
}

/// Acoustic material properties for absorption and reflection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AcousticMaterial {
    /// Hard reflective surface (concrete, metal, water, marble)
    Hard,
    /// Medium reflective surface (drywall, wood)
    Medium,
    /// Absorptive surface (carpet, fabric, foam, curtains)
    Soft,
    /// Highly absorptive (acoustic panels, baffles)
    HighlyAbsorptive,
}

impl AcousticMaterial {
    /// Absorption coefficient (0.0 = fully reflective, 1.0 = fully absorptive)
    pub fn absorption(&self) -> f32 {
        match self {
            AcousticMaterial::Hard => 0.05,
            AcousticMaterial::Medium => 0.25,
            AcousticMaterial::Soft => 0.6,
            AcousticMaterial::HighlyAbsorptive => 0.9,
        }
    }

    /// High-frequency damping (0.0 = preserve highs, 1.0 = full damping)
    pub fn high_frequency_damping(&self) -> f32 {
        match self {
            AcousticMaterial::Hard => 0.1,
            AcousticMaterial::Medium => 0.4,
            AcousticMaterial::Soft => 0.7,
            AcousticMaterial::HighlyAbsorptive => 0.95,
        }
    }
}

/// Spatial audio emitter with advanced acoustic properties.
///
/// Extends basic `AudioEmitter` with occlusion, obstruction, and material properties.
#[derive(Debug, Clone)]
pub struct SpatialAudioEmitter {
    /// Base emitter properties
    pub emitter: AudioEmitter,
    /// Acoustic material of the source (affects how sound reflects from it)
    pub material: AcousticMaterial,
    /// Radius within which sounds are fully occluded (blocked completely)
    pub occlusion_radius: f32,
    /// Radius within which sounds are partially obstructed (filtered)
    pub obstruction_radius: f32,
    /// Current occlusion coefficient (0.0 = no occlusion, 1.0 = fully occluded)
    pub occlusion_coefficient: f32,
    /// Current obstruction coefficient (0.0 = no obstruction, 1.0 = fully obstructed)
    pub obstruction_coefficient: f32,
}

impl SpatialAudioEmitter {
    pub fn new(position: Vec3) -> Self {
        Self {
            emitter: AudioEmitter::new(position),
            material: AcousticMaterial::Hard,
            occlusion_radius: 5.0,
            obstruction_radius: 15.0,
            occlusion_coefficient: 0.0,
            obstruction_coefficient: 0.0,
        }
    }

    pub fn with_material(mut self, material: AcousticMaterial) -> Self {
        self.material = material;
        self
    }

    pub fn with_occlusion_radius(mut self, radius: f32) -> Self {
        self.occlusion_radius = radius.max(0.1);
        self
    }

    pub fn with_obstruction_radius(mut self, radius: f32) -> Self {
        self.obstruction_radius = radius.max(0.1);
        self
    }
}

impl Default for SpatialAudioEmitter {
    fn default() -> Self {
        Self::new(Vec3::ZERO)
    }
}

/// Reverb zone defining acoustic space (room, cave, etc).
///
/// Acts as a trigger volume that applies reverb effect when the listener is inside.
#[derive(Debug, Clone)]
pub struct ReverbZone {
    /// Name of the preset (e.g., "cavern", "bathroom", "cathedral")
    pub preset_name: String,
    /// Approximate room volume in cubic meters (affects reverb decay)
    pub room_volume: f32,
    /// Surface area in square meters (affects reverb damping)
    pub surface_area: f32,
    /// Blend factor when multiple zones overlap (0.0-1.0)
    pub blend_distance: f32,
    /// Current blend weight (used for interpolation between zones)
    pub current_weight: f32,
    /// Whether listener is currently inside this zone
    pub listener_inside: bool,
}

impl ReverbZone {
    pub fn new(preset_name: &str) -> Self {
        Self {
            preset_name: preset_name.to_string(),
            room_volume: 100.0,
            surface_area: 200.0,
            blend_distance: 2.0,
            current_weight: 0.0,
            listener_inside: false,
        }
    }

    pub fn with_volume(mut self, volume: f32) -> Self {
        self.room_volume = volume.max(1.0);
        self
    }

    pub fn with_surface_area(mut self, area: f32) -> Self {
        self.surface_area = area.max(1.0);
        self
    }
}

impl Default for ReverbZone {
    fn default() -> Self {
        Self::new("default")
    }
}

/// Preset reverb parameters for common acoustic spaces.
#[derive(Debug, Clone, Copy)]
pub struct ReverbPreset {
    /// Early reflection delay (milliseconds)
    pub early_delay: f32,
    /// Reverb decay time (seconds)
    pub decay_time: f32,
    /// Reverb level (-80.0 to 0.0 dB)
    pub level: f32,
    /// High-frequency damping (0.0-1.0)
    pub damping: f32,
    /// Diffusion (0.0-1.0, how spread out reflections are)
    pub diffusion: f32,
    /// Width (0.0 = mono, 1.0 = full stereo)
    pub width: f32,
}

impl ReverbPreset {
    /// Small bathroom-like space
    pub fn bathroom() -> Self {
        Self {
            early_delay: 5.0,
            decay_time: 1.5,
            level: -3.0,
            damping: 0.5,
            diffusion: 0.8,
            width: 0.5,
        }
    }

    /// Large cavern or cave
    pub fn cavern() -> Self {
        Self {
            early_delay: 30.0,
            decay_time: 4.0,
            level: -10.0,
            damping: 0.3,
            diffusion: 0.9,
            width: 1.0,
        }
    }

    /// Cathedral or large hall
    pub fn cathedral() -> Self {
        Self {
            early_delay: 50.0,
            decay_time: 8.0,
            level: -15.0,
            damping: 0.2,
            diffusion: 0.95,
            width: 1.0,
        }
    }

    /// Small room or office
    pub fn small_room() -> Self {
        Self {
            early_delay: 8.0,
            decay_time: 0.8,
            level: 0.0,
            damping: 0.7,
            diffusion: 0.6,
            width: 0.4,
        }
    }

    /// Highly absorptive (like a studio or carpeted room)
    pub fn studio() -> Self {
        Self {
            early_delay: 3.0,
            decay_time: 0.3,
            level: 3.0,
            damping: 0.9,
            diffusion: 0.2,
            width: 0.2,
        }
    }

    /// Outdoor or very large space
    pub fn outdoor() -> Self {
        Self {
            early_delay: 0.0,
            decay_time: 0.1,
            level: 6.0,
            damping: 1.0,
            diffusion: 0.0,
            width: 0.0,
        }
    }

    /// Retrieves a preset by name
    pub fn from_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "bathroom" => Self::bathroom(),
            "cavern" => Self::cavern(),
            "cathedral" => Self::cathedral(),
            "small_room" => Self::small_room(),
            "studio" => Self::studio(),
            "outdoor" => Self::outdoor(),
            _ => Self::small_room(), // default
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emitter_defaults() {
        let e = AudioEmitter::new(Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(e.volume, 1.0);
        assert!(!e.playing);
        assert!(!e.looping);
        assert!(e.sound_handle.is_none());
    }

    #[test]
    fn emitter_play_stop() {
        let mut e = AudioEmitter::default();
        e.play();
        assert!(e.playing);
        e.stop();
        assert!(!e.playing);
    }

    #[test]
    fn listener_defaults() {
        let l = AudioListener::default();
        assert!((l.forward - Vec3::NEG_Z).length() < 0.001);
        assert!((l.up - Vec3::Y).length() < 0.001);
    }

    #[test]
    fn acoustic_material_absorption() {
        assert_eq!(AcousticMaterial::Hard.absorption(), 0.05);
        assert_eq!(AcousticMaterial::Soft.absorption(), 0.6);
    }

    #[test]
    fn reverb_preset_loading() {
        let preset = ReverbPreset::from_name("cathedral");
        assert!(preset.decay_time > 5.0);
    }
}
