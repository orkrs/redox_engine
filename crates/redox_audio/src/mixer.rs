//! MetaSounds-lite mixer graph (Uplift v2 Phase 8).
//!
//! Hierarchical buses (Music / Sfx / Voice / Ui → Master) with linear gain and
//! mute. [`AudioContext`] owns a live copy, creates matching kira sub-tracks when
//! a device is available, and routes [`AudioContext::play_sfx`] /
//! [`AudioContext::play_one_shot`] / [`AudioContext::play_loop`] through them.
//!
//! Without an audio device the graph still evaluates `routed_gain` correctly so
//! gameplay can call `play_sfx` unconditionally (it becomes a no-op).

use std::collections::HashMap;

/// Identifies a mixer bus in the MetaSounds-lite hierarchy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MixerBusId {
    Master,
    Music,
    Sfx,
    Voice,
    Ui,
}

impl MixerBusId {
    pub fn all_sub_buses() -> [MixerBusId; 4] {
        [Self::Music, Self::Sfx, Self::Voice, Self::Ui]
    }

    pub fn default_volume(self) -> f32 {
        match self {
            Self::Master => 1.0,
            Self::Music => 0.7,
            Self::Sfx => 1.0,
            Self::Voice => 1.0,
            Self::Ui => 0.85,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Master => "master",
            Self::Music => "music",
            Self::Sfx => "sfx",
            Self::Voice => "voice",
            Self::Ui => "ui",
        }
    }
}

/// Single mixer bus with linear volume and mute state.
#[derive(Clone, Debug)]
pub struct MixerBus {
    pub id: MixerBusId,
    pub volume: f32,
    pub muted: bool,
}

impl MixerBus {
    pub fn new(id: MixerBusId) -> Self {
        Self {
            id,
            volume: id.default_volume(),
            muted: false,
        }
    }

    /// Effective linear gain after mute.
    pub fn effective_gain(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            self.volume.clamp(0.0, 2.0)
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 2.0);
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }
}

/// How a clip should be scheduled on a bus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayMode {
    OneShot,
    Loop,
}

/// Request to play a named or handle-backed clip through the mixer.
#[derive(Clone, Debug)]
pub struct PlayRequest {
    pub bus: MixerBusId,
    pub mode: PlayMode,
    /// Per-voice gain multiplied by bus routing.
    pub gain: f32,
}

impl PlayRequest {
    pub fn one_shot(bus: MixerBusId, gain: f32) -> Self {
        Self {
            bus,
            mode: PlayMode::OneShot,
            gain,
        }
    }

    pub fn looping(bus: MixerBusId, gain: f32) -> Self {
        Self {
            bus,
            mode: PlayMode::Loop,
            gain,
        }
    }

    pub fn sfx(gain: f32) -> Self {
        Self::one_shot(MixerBusId::Sfx, gain)
    }
}

/// Converts linear amplitude to decibels (dB FS).
pub fn linear_to_db(linear: f32) -> f32 {
    if linear <= 1e-5 {
        -100.0
    } else {
        20.0 * linear.log10()
    }
}

/// Converts decibels (dB FS) to linear amplitude.
pub fn db_to_linear(db: f32) -> f32 {
    if db <= -100.0 {
        0.0
    } else {
        10.0_f32.powf(db / 20.0)
    }
}

/// Sidechain ducking configuration and runtime envelope.
#[derive(Clone, Debug)]
pub struct DuckingController {
    /// Bus whose volume will be attenuated (typically Music).
    pub target_bus: MixerBusId,
    /// Bus that triggers attenuation (e.g., Voice or Dialogue).
    pub trigger_bus: MixerBusId,
    /// Attenuation depth in decibels (e.g., -6.0 or -12.0).
    pub duck_depth_db: f32,
    /// Attack duration in seconds (time to reach ducked level).
    pub attack_time: f32,
    /// Release duration in seconds (time to recover to unity).
    pub release_time: f32,
    /// Current linear duck multiplier (1.0 = unity, 0.5 = -6 dB, etc.)
    pub current_gain: f32,
    /// Whether ducking trigger is active this frame.
    pub active: bool,
}

impl DuckingController {
    pub fn new(target_bus: MixerBusId, trigger_bus: MixerBusId, duck_depth_db: f32) -> Self {
        Self {
            target_bus,
            trigger_bus,
            duck_depth_db,
            attack_time: 0.1,
            release_time: 0.4,
            current_gain: 1.0,
            active: false,
        }
    }

    /// Advances the envelope by delta time.
    pub fn update(&mut self, dt: f32) {
        let target_linear = if self.active {
            db_to_linear(self.duck_depth_db)
        } else {
            1.0
        };

        let rate = if self.active {
            if self.attack_time > 1e-4 {
                dt / self.attack_time
            } else {
                1.0
            }
        } else {
            if self.release_time > 1e-4 {
                dt / self.release_time
            } else {
                1.0
            }
        };

        if self.current_gain < target_linear {
            self.current_gain = (self.current_gain + rate).min(target_linear);
        } else if self.current_gain > target_linear {
            self.current_gain = (self.current_gain - rate).max(target_linear);
        }
    }
}

/// Hierarchical mixer routing all gameplay buses into master.
#[derive(Clone, Debug)]
pub struct MixerGraph {
    pub master: MixerBus,
    pub music: MixerBus,
    pub sfx: MixerBus,
    pub voice: MixerBus,
    pub ui: MixerBus,
    pub ducking: Option<DuckingController>,
}

impl Default for MixerGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl MixerGraph {
    pub fn new() -> Self {
        Self {
            master: MixerBus::new(MixerBusId::Master),
            music: MixerBus::new(MixerBusId::Music),
            sfx: MixerBus::new(MixerBusId::Sfx),
            voice: MixerBus::new(MixerBusId::Voice),
            ui: MixerBus::new(MixerBusId::Ui),
            ducking: None,
        }
    }

    pub fn with_voice_ducking(mut self, depth_db: f32) -> Self {
        self.ducking = Some(DuckingController::new(
            MixerBusId::Music,
            MixerBusId::Voice,
            depth_db,
        ));
        self
    }

    pub fn update(&mut self, dt: f32) {
        if let Some(ducking) = &mut self.ducking {
            ducking.update(dt);
        }
    }

    pub fn bus_mut(&mut self, id: MixerBusId) -> &mut MixerBus {
        match id {
            MixerBusId::Master => &mut self.master,
            MixerBusId::Music => &mut self.music,
            MixerBusId::Sfx => &mut self.sfx,
            MixerBusId::Voice => &mut self.voice,
            MixerBusId::Ui => &mut self.ui,
        }
    }

    pub fn bus(&self, id: MixerBusId) -> &MixerBus {
        match id {
            MixerBusId::Master => &self.master,
            MixerBusId::Music => &self.music,
            MixerBusId::Sfx => &self.sfx,
            MixerBusId::Voice => &self.voice,
            MixerBusId::Ui => &self.ui,
        }
    }

    /// Combined gain for routing `source` through master, including active ducking.
    pub fn routed_gain(&self, source: MixerBusId) -> f32 {
        let base = if source == MixerBusId::Master {
            self.master.effective_gain()
        } else {
            self.bus(source).effective_gain() * self.master.effective_gain()
        };

        if let Some(ducking) = &self.ducking {
            if ducking.target_bus == source {
                return base * ducking.current_gain;
            }
        }
        base
    }

    /// Final voice gain = request gain × routed bus gain.
    pub fn voice_gain(&self, request: &PlayRequest) -> f32 {
        (request.gain.max(0.0) * self.routed_gain(request.bus)).clamp(0.0, 2.0)
    }

    /// Snapshot of all bus effective gains (for debug UI / syncing kira tracks).
    pub fn gains_snapshot(&self) -> HashMap<MixerBusId, f32> {
        let mut map = HashMap::new();
        map.insert(MixerBusId::Master, self.master.effective_gain());
        for id in MixerBusId::all_sub_buses() {
            map.insert(id, self.routed_gain(id));
        }
        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decibel_conversions() {
        assert_eq!(linear_to_db(1.0), 0.0);
        assert!((linear_to_db(0.5) - (-6.0205)).abs() < 0.01);
        assert!((db_to_linear(0.0) - 1.0).abs() < 1e-4);
        assert!((db_to_linear(-6.0205) - 0.5).abs() < 0.01);
    }

    #[test]
    fn ducking_attenuates_music() {
        let mut g = MixerGraph::new().with_voice_ducking(-6.0);
        let normal_music = g.routed_gain(MixerBusId::Music);

        // Trigger voice
        if let Some(duck) = &mut g.ducking {
            duck.active = true;
        }
        g.update(0.1); // attack complete

        let ducked_music = g.routed_gain(MixerBusId::Music);
        assert!(ducked_music < normal_music);
        assert!((ducked_music / normal_music - 0.5).abs() < 0.05);
    }

    #[test]
    fn routed_gain_respects_mute() {
        let mut g = MixerGraph::new();
        g.sfx.set_volume(0.5);
        assert!((g.routed_gain(MixerBusId::Sfx) - 0.5).abs() < 1e-4);
        g.master.set_muted(true);
        assert_eq!(g.routed_gain(MixerBusId::Sfx), 0.0);
    }

    #[test]
    fn voice_gain_scales_request() {
        let g = MixerGraph::new();
        let req = PlayRequest::sfx(0.5);
        assert!((g.voice_gain(&req) - 0.5).abs() < 1e-4);
    }
}
