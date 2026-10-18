//! Audio context — ECS resource wrapping kira's AudioManager + MetaSounds-lite mixer.
//!
//! # Wiring with the mixer
//! 1. `let mut audio = AudioContext::new();` (or [`with_audio`] into a `World`)
//! 2. Register clips: `audio.register_sfx("dash", handle);` after
//!    [`sync_assets_to_audio`] / [`AudioContext::add_sound_from_asset`]
//! 3. On gameplay events: `audio.play_sfx("dash", 1.0);`
//! 4. Optionally tweak buses: `audio.mixer.bus_mut(MixerBusId::Sfx).set_volume(0.8);`
//!    then `audio.sync_mixer();` so kira track volumes match.
//!
//! If no audio device is present, play APIs return `None` but the mixer graph
//! still updates — safe to call from Prism Vault dash / pickup paths.

use std::collections::HashMap;
use std::io::Cursor;

use kira::effect::filter::{FilterBuilder, FilterHandle, FilterMode};
use kira::effect::reverb::{ReverbBuilder, ReverbHandle};
use kira::listener::ListenerHandle;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle, StaticSoundSettings};
use kira::track::{MainTrackBuilder, SpatialTrackBuilder, TrackBuilder, TrackHandle};
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Mix, Tween};
use redox_asset::{AssetId, Handle};
use redox_ecs::{Entity, World};

use crate::asset_types::AudioData;
use crate::components::ReverbPreset;
use crate::mixer::{MixerBusId, MixerGraph, PlayMode, PlayRequest};
use crate::music::MusicDirector;
use crate::music_play::DEFAULT_MUSIC_CROSSFADE_SECONDS;
use crate::voices::{VoiceBudget, VoiceDecision};

/// Default number of cue voices that may sound at once.
const DEFAULT_CUE_VOICES: usize = 16;
/// Default assumed length in seconds of a started cue voice.
const DEFAULT_CUE_VOICE_SECONDS: f32 = 1.0;

/// ECS resource that manages the audio engine and MetaSounds-lite buses.
pub struct AudioContext {
    manager: Option<AudioManager>,
    /// Live MetaSounds-lite graph (Music / Sfx / Voice / Ui → Master).
    pub mixer: MixerGraph,
    /// Kira sub-tracks keyed by bus (Master uses the main track).
    bus_tracks: HashMap<MixerBusId, TrackHandle>,
    listener: Option<ListenerHandle>,
    lowpass: Option<FilterHandle>,
    reverb: Option<ReverbHandle>,
    /// Named one-shot / loop aliases → cached asset ids.
    named_sounds: HashMap<String, AssetId>,
    /// Master volume (0.0 → 1.0) applied to the main track.
    pub master_volume: f64,
    /// Cached decoded sounds by asset handle id (from [`Handle<AudioData>`]).
    sound_cache: HashMap<AssetId, StaticSoundData>,
    /// Occlusion coefficient per emitter entity (0.0 = clear, 1.0 = fully occluded).
    occlusion_by_emitter: HashMap<Entity, f32>,
    /// Priority-based limit on simultaneous cue voices (see `set_cue_voice_limit`).
    cue_voices: VoiceBudget,
    /// Playing sound handle of each cue voice that is still held by the budget, by voice id.
    pub(crate) cue_handles: HashMap<u64, StaticSoundHandle>,
    /// Assumed length in seconds of a started cue voice (see `set_cue_voice_seconds`).
    cue_voice_seconds: f32,
    /// Time of the latest cue voice request; used by `set_cue_voice_limit` to expire voices.
    cue_voice_now: f32,
    /// Id given to the next cue voice request; increases monotonically.
    next_voice_id: u64,
    /// Music crossfade state (see `play_music`).
    pub(crate) music: MusicDirector,
}

impl AudioContext {
    /// Initialises the audio context and attempts to create bus sub-tracks.
    ///
    /// If the audio backend fails (e.g., no audio device), the context is
    /// created in a degraded mode where all play operations are no-ops.
    pub fn new() -> Self {
        let mut main_track = MainTrackBuilder::new();
        let lowpass = main_track.add_effect(
            FilterBuilder::new()
                .mode(FilterMode::LowPass)
                .cutoff(20_000.0),
        );
        // Reverb stays fully dry until a reverb zone becomes active.
        let reverb = main_track.add_effect(ReverbBuilder::new().mix(Mix::DRY));
        let mut settings = AudioManagerSettings::default();
        settings.main_track_builder = main_track;

        let mut manager = AudioManager::<DefaultBackend>::new(settings)
            .map_err(|e| {
                log::warn!("Audio backend unavailable: {}. Running without audio.", e);
                e
            })
            .ok();

        let mixer = MixerGraph::new();
        let mut bus_tracks = HashMap::new();
        let mut listener = None;
        if let Some(m) = manager.as_mut() {
            for id in MixerBusId::all_sub_buses() {
                let vol = mixer.bus(id).effective_gain();
                match m.add_sub_track(TrackBuilder::new().volume(gain_to_db(vol))) {
                    Ok(track) => {
                        bus_tracks.insert(id, track);
                    }
                    Err(e) => {
                        log::warn!("Failed to create mixer bus {:?}: {e}", id);
                    }
                }
            }
            m.main_track()
                .set_volume(gain_to_db(mixer.master.effective_gain()), Tween::default());
            match m.add_listener(redox_math::Vec3::ZERO, redox_math::Quat::IDENTITY) {
                Ok(l) => listener = Some(l),
                Err(e) => log::warn!("Failed to create spatial listener: {e}"),
            }
        }

        Self {
            manager,
            mixer,
            bus_tracks,
            listener,
            lowpass: Some(lowpass),
            reverb: Some(reverb),
            named_sounds: HashMap::new(),
            master_volume: 1.0,
            sound_cache: HashMap::new(),
            occlusion_by_emitter: HashMap::new(),
            cue_voices: VoiceBudget::new(DEFAULT_CUE_VOICES),
            cue_handles: HashMap::new(),
            cue_voice_seconds: DEFAULT_CUE_VOICE_SECONDS,
            cue_voice_now: 0.0,
            next_voice_id: 0,
            music: MusicDirector::new(DEFAULT_MUSIC_CROSSFADE_SECONDS),
        }
    }

    /// Sets how many cue voices may sound at once. Default 16. Voices above the new limit are
    /// stopped, weakest first. The clock is the time of the latest cue request.
    pub fn set_cue_voice_limit(&mut self, max_voices: usize) {
        let evicted = self
            .cue_voices
            .set_max_voices(max_voices, self.cue_voice_now);
        for id in evicted {
            self.stop_cue_voice(id);
        }
    }

    /// Sets how long, in seconds, a started cue occupies its voice. Default 1.0. Non-finite
    /// or non-positive values are ignored.
    pub fn set_cue_voice_seconds(&mut self, seconds: f32) {
        if seconds.is_finite() && seconds > 0.0 {
            self.cue_voice_seconds = seconds;
        }
    }

    /// Asks the cue voice budget for a slot for a cue of `priority` starting at `now`.
    ///
    /// Assigns the next voice id from the context's counter, so every request gets a fresh id,
    /// and returns it with the decision. When the decision steals a slot, the stolen voice's
    /// sound is stopped. Handles of voices that have ended are dropped first.
    pub(crate) fn request_cue_voice(&mut self, priority: u8, now: f32) -> (u64, VoiceDecision) {
        self.cue_voice_now = now;
        let live = self.cue_voices.ids(now);
        self.cue_handles.retain(|id, _| live.contains(id));
        let voice_id = self.next_voice_id;
        self.next_voice_id = self.next_voice_id.wrapping_add(1);
        let decision = self
            .cue_voices
            .request(voice_id, priority, now, self.cue_voice_seconds);
        if let VoiceDecision::Steal(victim) = decision {
            self.stop_cue_voice(victim);
        }
        (voice_id, decision)
    }

    /// Stops the sound of cue voice `id` (if it has a handle) and forgets the handle.
    fn stop_cue_voice(&mut self, id: u64) {
        if let Some(mut handle) = self.cue_handles.remove(&id) {
            handle.stop(Tween::default());
        }
    }

    /// Returns `true` if the audio backend is available.
    pub fn is_available(&self) -> bool {
        self.manager.is_some()
    }

    /// Returns a mutable reference to the internal kira `AudioManager`, if available.
    pub fn manager_mut(&mut self) -> Option<&mut AudioManager> {
        self.manager.as_mut()
    }

    /// Pushes current [`MixerGraph`] volumes onto kira tracks / main track.
    pub fn sync_mixer(&mut self) {
        let master_gain = self.mixer.master.effective_gain() * self.master_volume as f32;
        if let Some(m) = self.manager.as_mut() {
            m.main_track()
                .set_volume(gain_to_db(master_gain), Tween::default());
        }
        for id in MixerBusId::all_sub_buses() {
            let gain = self.mixer.bus(id).effective_gain();
            if let Some(track) = self.bus_tracks.get_mut(&id) {
                track.set_volume(gain_to_db(gain), Tween::default());
            }
        }
    }

    /// Sets the master volume (clamped to 0.0–1.0) and syncs the mixer.
    pub fn set_master_volume(&mut self, volume: f64) {
        self.master_volume = volume.clamp(0.0, 1.0);
        self.mixer.master.set_volume(self.master_volume as f32);
        self.sync_mixer();
    }

    /// Registers a named alias for a cached sound (e.g. `"dash"`, `"pickup"`).
    pub fn register_sfx(&mut self, name: impl Into<String>, handle: Handle<AudioData>) {
        self.named_sounds.insert(name.into(), handle.id());
    }

    /// Registers raw decoded data under a name (useful without the asset manager).
    pub fn register_sfx_data(&mut self, name: impl Into<String>, data: StaticSoundData) -> AssetId {
        let name = name.into();
        let id = AssetId(fxhash_name(&name));
        self.sound_cache.insert(id, data);
        self.named_sounds.insert(name, id);
        id
    }

    /// Plays a registered one-shot on the Sfx bus.
    ///
    /// ```ignore
    /// audio.play_sfx("dash", 1.0);
    /// audio.play_sfx("pickup", 0.8);
    /// ```
    pub fn play_sfx(&mut self, name: &str, gain: f32) -> Option<StaticSoundHandle> {
        let id = *self.named_sounds.get(name)?;
        self.play_asset_id(id, PlayRequest::sfx(gain), 1.0)
    }

    /// Plays a cached asset as a one-shot on `bus` with `gain`.
    pub fn play_one_shot(
        &mut self,
        handle: Handle<AudioData>,
        bus: MixerBusId,
        gain: f32,
    ) -> Option<StaticSoundHandle> {
        self.play_asset_id(handle.id(), PlayRequest::one_shot(bus, gain), 1.0)
    }

    /// Plays a cached asset looping on `bus` with `gain`.
    pub fn play_loop(
        &mut self,
        handle: Handle<AudioData>,
        bus: MixerBusId,
        gain: f32,
    ) -> Option<StaticSoundHandle> {
        self.play_asset_id(handle.id(), PlayRequest::looping(bus, gain), 1.0)
    }

    /// Plays by name with an explicit [`PlayRequest`] (bus + mode + gain).
    pub fn play_named(&mut self, name: &str, request: PlayRequest) -> Option<StaticSoundHandle> {
        self.play_named_pitched(name, request, 1.0)
    }

    /// Plays by name with an explicit [`PlayRequest`] and a playback rate.
    ///
    /// `playback_rate` is a speed factor where `1.0` is the original pitch. Changing it
    /// changes speed and pitch together, as kira does.
    pub fn play_named_pitched(
        &mut self,
        name: &str,
        request: PlayRequest,
        playback_rate: f64,
    ) -> Option<StaticSoundHandle> {
        let id = *self.named_sounds.get(name)?;
        self.play_asset_id(id, request, playback_rate)
    }

    fn play_asset_id(
        &mut self,
        id: AssetId,
        request: PlayRequest,
        playback_rate: f64,
    ) -> Option<StaticSoundHandle> {
        let voice_gain = self.mixer.voice_gain(&request);
        if voice_gain <= 0.0001 {
            return None;
        }
        let mut data = self.sound_cache.get(&id)?.clone();
        let mut settings = StaticSoundSettings::new()
            .volume(gain_to_db(voice_gain))
            .playback_rate(playback_rate);
        if request.mode == PlayMode::Loop {
            settings = settings.loop_region(0.0..);
        }
        data.settings = settings;
        if request.bus != MixerBusId::Master
            && let Some(track) = self.bus_tracks.get_mut(&request.bus)
        {
            return track.play(data).ok();
        }
        self.play_sound(data)
    }

    /// Plays a static sound and returns a handle to the playing sound.
    pub fn play_sound(&mut self, data: StaticSoundData) -> Option<StaticSoundHandle> {
        self.manager.as_mut().and_then(|m| m.play(data).ok())
    }

    /// Plays a static sound with spatial settings.
    pub fn play_spatial(
        &mut self,
        data: StaticSoundData,
        position: redox_math::Vec3,
        max_distance: f32,
    ) -> Option<StaticSoundHandle> {
        let (Some(manager), Some(listener)) = (self.manager.as_mut(), self.listener.as_ref())
        else {
            return self.play_sound(data);
        };
        let mut track = manager
            .add_spatial_sub_track(
                listener,
                position,
                SpatialTrackBuilder::new()
                    .distances((0.1, max_distance.max(0.11)))
                    .persist_until_sounds_finish(true),
            )
            .ok()?;
        track.play(data).ok()
    }

    /// Moves the Kira listener (ears) to match the game camera / player.
    pub fn set_listener_pose(&mut self, position: redox_math::Vec3, orientation: redox_math::Quat) {
        if let Some(listener) = self.listener.as_mut() {
            listener.set_position(position, Tween::default());
            listener.set_orientation(orientation, Tween::default());
        }
    }

    /// Applies a low-pass filter to the main track (for occlusion/obstruction effect).
    pub fn set_lowpass_filter(&mut self, cutoff_hz: f32) {
        if let Some(filter) = self.lowpass.as_mut() {
            filter.set_cutoff(f64::from(cutoff_hz.max(20.0)), Tween::default());
        }
    }

    /// Applies the occlusion curve to the main-track lowpass (muffled when blocked).
    pub fn apply_occlusion_lowpass(&mut self, occlusion: f32) {
        self.set_lowpass_filter(Self::lowpass_cutoff_from_occlusion(
            occlusion.clamp(0.0, 1.0),
        ));
    }

    /// Applies reverb decay (seconds) and damping (0..1) to the main-track reverb.
    pub fn set_reverb(&mut self, decay: f32, damping: f32) {
        let Some(reverb) = self.reverb.as_mut() else {
            return;
        };
        let tween = reverb_tween();
        reverb.set_feedback(feedback_from_decay(decay), tween);
        reverb.set_damping(f64::from(damping.clamp(0.0, 1.0)), tween);
    }

    /// Drives the main-track reverb from the blended zone preset.
    ///
    /// `None` (listener outside every zone) fades the reverb back to fully dry.
    pub fn apply_reverb(&mut self, preset: Option<&ReverbPreset>) {
        let Some(reverb) = self.reverb.as_mut() else {
            return;
        };
        let params = preset.map(ReverbParams::from_preset).unwrap_or_default();
        let tween = reverb_tween();
        reverb.set_feedback(params.feedback, tween);
        reverb.set_damping(params.damping, tween);
        reverb.set_stereo_width(params.stereo_width, tween);
        reverb.set_mix(Mix(params.mix as f32), tween);
    }

    /// Sets the occlusion coefficient for an emitter entity.
    pub fn set_emitter_occlusion(&mut self, entity: Entity, coefficient: f32) {
        let c = coefficient.clamp(0.0, 1.0);
        if c > 0.0001 {
            self.occlusion_by_emitter.insert(entity, c);
        } else {
            self.occlusion_by_emitter.remove(&entity);
        }
    }

    /// Returns the current occlusion coefficient for an emitter (0.0 if unknown or clear).
    pub fn get_emitter_occlusion(&self, entity: Entity) -> f32 {
        self.occlusion_by_emitter
            .get(&entity)
            .copied()
            .unwrap_or(0.0)
    }

    /// Volume multiplier from occlusion (1.0 = no occlusion, lower when occluded).
    pub fn volume_multiplier_from_occlusion(occlusion: f32) -> f32 {
        (1.0 - occlusion * 0.7).max(0.0)
    }

    /// Suggested low-pass cutoff in Hz from occlusion.
    pub fn lowpass_cutoff_from_occlusion(occlusion: f32) -> f32 {
        20000.0 * (1.0 - occlusion).max(0.0) + 400.0 * occlusion
    }

    /// Updates spatial parameters for an emitter (volume, pan) based on listener position.
    pub fn update_spatial_parameters(
        &self,
        emitter_pos: redox_math::Vec3,
        listener_pos: redox_math::Vec3,
        listener_forward: redox_math::Vec3,
        listener_up: redox_math::Vec3,
        max_distance: f32,
    ) -> (f32, f32) {
        let to_emitter = emitter_pos - listener_pos;
        let distance = to_emitter.length();

        let volume = if distance >= max_distance {
            0.0
        } else if distance < 0.1 {
            1.0
        } else {
            ((max_distance - distance) / max_distance).clamp(0.0, 1.0)
        };

        let listener_right = listener_forward.cross(listener_up).normalize();
        let pan = if to_emitter.length_squared() < 1e-6 {
            0.0
        } else {
            to_emitter.normalize().dot(listener_right).clamp(-1.0, 1.0)
        };

        (volume, pan)
    }

    /// Caches decoded sound from asset data and associates it with the handle.
    pub fn add_sound_from_asset(&mut self, handle: Handle<AudioData>, data: &[u8]) -> bool {
        if self.sound_cache.contains_key(&handle.id()) {
            return false;
        }
        let cursor = Cursor::new(data.to_vec());
        match StaticSoundData::from_cursor(cursor) {
            Ok(sound) => {
                self.sound_cache.insert(handle.id(), sound);
                true
            }
            Err(e) => {
                log::warn!("Failed to decode audio asset {:?}: {:?}", handle.id(), e);
                false
            }
        }
    }

    /// Returns whether the given handle has been synced (decoded and cached).
    pub fn has_sound(&self, handle: Handle<AudioData>) -> bool {
        self.sound_cache.contains_key(&handle.id())
    }

    /// Plays a cached sound by handle (main track, unity gain).
    pub fn play_sound_by_handle(&mut self, handle: Handle<AudioData>) -> Option<StaticSoundHandle> {
        let data = self.sound_cache.get(&handle.id())?.clone();
        self.play_sound(data)
    }

    /// Plays a cached sound by handle with spatial settings.
    pub fn play_spatial_by_handle(
        &mut self,
        handle: Handle<AudioData>,
        position: redox_math::Vec3,
        max_distance: f32,
    ) -> Option<StaticSoundHandle> {
        let data = self.sound_cache.get(&handle.id())?.clone();
        self.play_spatial(data, position, max_distance)
    }
}

/// Inserts [`AudioContext`] into the world if missing and returns a mutable borrow.
///
/// ```ignore
/// let audio = with_audio(&mut world);
/// audio.mixer.bus_mut(MixerBusId::Music).set_volume(0.6);
/// audio.sync_mixer();
/// audio.play_sfx("dash", 1.0);
/// ```
pub fn with_audio(world: &mut World) -> &mut AudioContext {
    if world.get_resource::<AudioContext>().is_none() {
        world.insert_resource(AudioContext::new());
    }
    world
        .get_resource_mut::<AudioContext>()
        .expect("AudioContext just inserted")
}

/// Syncs loaded [`AudioData`] assets from the asset manager into the audio context.
pub fn sync_assets_to_audio(
    audio_ctx: &mut AudioContext,
    asset_manager: &redox_asset::AssetManager,
    audio_handles: &[Handle<AudioData>],
) {
    for &handle in audio_handles {
        if audio_ctx.has_sound(handle) {
            continue;
        }
        if let Some(data) = asset_manager.get(handle) {
            audio_ctx.add_sound_from_asset(handle, &data.0);
        }
    }
}

impl Default for AudioContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Linear gain (0..=1+) -> kira decibels; silence maps to [`Decibels::SILENCE`].
fn gain_to_db(gain: f32) -> Decibels {
    if gain <= 0.001 {
        Decibels::SILENCE
    } else {
        Decibels(20.0 * gain.log10())
    }
}

fn fxhash_name(name: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in name.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    // Keep out of the low asset-id range used by the asset manager when possible.
    h | 0xA000_0000_0000_0000
}

/// Kira reverb parameters derived from a [`ReverbPreset`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReverbParams {
    /// Reverb feedback, 0.5 to 0.95 (longer decay at higher values).
    pub feedback: f64,
    /// High-frequency damping, 0 to 1.
    pub damping: f64,
    /// Stereo width, 0 (mono) to 1 (full).
    pub stereo_width: f64,
    /// Wet share, 0.0 = fully dry.
    pub mix: f64,
}

impl Default for ReverbParams {
    /// Fully dry: used when the listener is outside every reverb zone.
    fn default() -> Self {
        Self {
            feedback: 0.5,
            damping: 0.5,
            stereo_width: 1.0,
            mix: 0.0,
        }
    }
}

impl ReverbParams {
    /// Maps decay time to feedback, `level` (dB) to the wet mix.
    pub fn from_preset(preset: &ReverbPreset) -> Self {
        // Half of full scale at 0 dB keeps loud presets from drowning the dry signal.
        let wet = 10f64.powf(f64::from(preset.level) / 20.0).clamp(0.0, 1.0) * 0.5;
        Self {
            feedback: feedback_from_decay(preset.decay_time),
            damping: f64::from(preset.damping.clamp(0.0, 1.0)),
            stereo_width: f64::from(preset.width.clamp(0.0, 1.0)),
            mix: wet,
        }
    }
}

/// Decay time in seconds -> kira feedback (0.5 .. 0.95; 1.0 would ring forever).
fn feedback_from_decay(decay_seconds: f32) -> f64 {
    0.5 + 0.45 * f64::from((decay_seconds / 10.0).clamp(0.0, 1.0))
}

fn reverb_tween() -> Tween {
    Tween {
        duration: std::time::Duration::from_millis(250),
        ..Tween::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_creation_does_not_panic() {
        let ctx = AudioContext::new();
        let _ = ctx.is_available();
        assert!((ctx.mixer.routed_gain(MixerBusId::Sfx) - 1.0).abs() < 0.01);
    }

    #[test]
    fn master_volume_clamped() {
        let mut ctx = AudioContext::new();
        ctx.set_master_volume(2.0);
        assert!((ctx.master_volume - 1.0).abs() < 0.001);
        ctx.set_master_volume(-0.5);
        assert!((ctx.master_volume - 0.0).abs() < 0.001);
    }

    #[test]
    fn play_sfx_unknown_is_none() {
        let mut ctx = AudioContext::new();
        assert!(ctx.play_sfx("missing", 1.0).is_none());
    }

    #[test]
    fn with_audio_inserts_resource() {
        let mut world = World::new();
        let _ = with_audio(&mut world);
        assert!(world.get_resource::<AudioContext>().is_some());
    }

    #[test]
    fn reverb_params_scale_with_preset() {
        let cathedral = ReverbParams::from_preset(&ReverbPreset::cathedral());
        let studio = ReverbParams::from_preset(&ReverbPreset::studio());
        assert!(cathedral.feedback > studio.feedback);
        assert!(cathedral.feedback <= 0.95 && studio.feedback >= 0.5);
        assert!((0.0..=1.0).contains(&cathedral.mix));
    }

    #[test]
    fn no_zone_means_dry_reverb() {
        assert_eq!(ReverbParams::default().mix, 0.0);
    }

    #[test]
    fn lowpass_follows_occlusion() {
        let clear = AudioContext::lowpass_cutoff_from_occlusion(0.0);
        let blocked = AudioContext::lowpass_cutoff_from_occlusion(1.0);
        assert!(clear > 10_000.0);
        assert!(blocked < 1_000.0);
        let mut ctx = AudioContext::new();
        ctx.apply_occlusion_lowpass(0.8);
    }
}
