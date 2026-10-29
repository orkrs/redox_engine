//! Playback of [`SpatialAudioEmitter`]s through kira.
//!
//! A playing emitter with a cached sound gets its own spatial kira track (a "voice"). Every frame
//! the voice follows the emitter's position and its occlusion-adjusted volume. The voice is freed
//! when the emitter stops, loses its sound, is despawned, or its non-looping sound finishes.
//! Without an audio device the system does nothing harmful: no voice is started.

use std::collections::HashSet;
use std::time::Duration;

use kira::Tween;
use kira::sound::PlaybackState;
use kira::sound::static_sound::{StaticSoundHandle, StaticSoundSettings};
use kira::track::SpatialTrackHandle;
use redox_asset::Handle;
use redox_ecs::{Entity, World};
use redox_math::Vec3;

use crate::asset_types::AudioData;
use crate::components::{AudioEmitter, SpatialAudioEmitter};
use crate::context::{AudioContext, gain_to_db};

/// Fade used when a voice is stopped on purpose, so it does not click.
const VOICE_STOP_FADE: Duration = Duration::from_millis(30);

/// What the playback system should do for one emitter this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EmitterAction {
    /// Nothing to do: not playing and no voice.
    Idle,
    /// Start a voice (`playing`, a sound is set and cached, no voice yet).
    Start,
    /// A voice exists and should continue; update position and volume.
    Update,
    /// A voice exists but the emitter stopped (`playing == false`) or lost its sound: stop and free it.
    Stop,
}

/// `has_voice`: the context already holds a voice for this entity. `sound_ready`: the emitter has a
/// `sound_handle` and `AudioContext::has_sound` is true for it.
pub(crate) fn emitter_action(playing: bool, sound_ready: bool, has_voice: bool) -> EmitterAction {
    match (playing && sound_ready, has_voice) {
        (true, false) => EmitterAction::Start,
        (true, true) => EmitterAction::Update,
        (false, true) => EmitterAction::Stop,
        (false, false) => EmitterAction::Idle,
    }
}

/// Linear volume an emitter should play at: its own volume (clamped to 0 or more) with the
/// occlusion multiplier of `entity` applied. Delegates to [`AudioContext::emitter_volume`].
pub fn emitter_target_volume(ctx: &AudioContext, entity: Entity, emitter: &AudioEmitter) -> f32 {
    ctx.emitter_volume(entity, emitter.volume.max(0.0))
}

/// A spatial kira track playing one emitter's sound.
pub(crate) struct EmitterVoice {
    track: SpatialTrackHandle,
    sound: StaticSoundHandle,
}

impl AudioContext {
    /// Starts a spatial voice for `entity`. Returns `false` (and starts nothing) without a device,
    /// listener or cached sound, or when the entity already has a voice.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start_emitter_voice(
        &mut self,
        entity: Entity,
        sound: Handle<AudioData>,
        position: Vec3,
        max_distance: f32,
        volume: f32,
        pitch: f32,
        looping: bool,
    ) -> bool {
        if self.emitter_voices.contains_key(&entity) {
            return false;
        }
        let Some(mut data) = self.cached_sound(sound) else {
            return false;
        };
        let Some(mut track) = self.spatial_track(position, max_distance) else {
            return false;
        };
        let mut settings = StaticSoundSettings::new()
            .volume(gain_to_db(volume))
            .playback_rate(f64::from(pitch.max(0.01)));
        if looping {
            settings = settings.loop_region(0.0..);
        }
        data.settings = settings;
        let Ok(sound_handle) = track.play(data) else {
            return false;
        };
        self.emitter_voices.insert(
            entity,
            EmitterVoice {
                track,
                sound: sound_handle,
            },
        );
        true
    }

    /// Moves the voice of `entity` to `position` and sets its linear `volume`. No-op without a voice.
    pub(crate) fn update_emitter_voice(&mut self, entity: Entity, position: Vec3, volume: f32) {
        if let Some(voice) = self.emitter_voices.get_mut(&entity) {
            voice.track.set_position(position, Tween::default());
            voice.track.set_volume(gain_to_db(volume), Tween::default());
        }
    }

    /// Stops (with a short fade) and drops the voice of `entity`.
    pub(crate) fn stop_emitter_voice(&mut self, entity: Entity) {
        if let Some(mut voice) = self.emitter_voices.remove(&entity) {
            voice.sound.stop(voice_stop_tween());
        }
    }

    /// True when `entity` has a voice.
    pub fn has_emitter_voice(&self, entity: Entity) -> bool {
        self.emitter_voices.contains_key(&entity)
    }

    /// Drops voices of entities missing from `alive`, and voices whose sound has finished.
    /// Returns the entities whose sound finished, so the caller can clear their `playing` flag.
    pub(crate) fn prune_emitter_voices(&mut self, alive: &HashSet<Entity>) -> Vec<Entity> {
        let mut finished = Vec::new();
        self.emitter_voices.retain(|entity, voice| {
            if !alive.contains(entity) {
                voice.sound.stop(voice_stop_tween());
                return false;
            }
            if matches!(voice.sound.state(), PlaybackState::Stopped) {
                finished.push(*entity);
                return false;
            }
            true
        });
        finished
    }
}

fn voice_stop_tween() -> Tween {
    Tween {
        duration: VOICE_STOP_FADE,
        ..Tween::default()
    }
}

/// Copy of the emitter data the playback system needs for one frame.
struct EmitterSnapshot {
    entity: Entity,
    emitter: AudioEmitter,
}

/// Plays `SpatialAudioEmitter`s: one kira spatial track per playing emitter, with position and
/// volume (including occlusion) refreshed every frame. Run after `occlusion_raycast_system`.
pub fn emitter_playback_system(world: &mut World) {
    if world.get_resource::<AudioContext>().is_none() {
        return;
    }
    let snapshots: Vec<EmitterSnapshot> = world
        .all_entities()
        .filter_map(|entity| {
            let spatial = world.get_component::<SpatialAudioEmitter>(entity)?;
            Some(EmitterSnapshot {
                entity,
                emitter: spatial.emitter.clone(),
            })
        })
        .collect();
    let alive: HashSet<Entity> = snapshots.iter().map(|s| s.entity).collect();

    let Some(ctx) = world.get_resource_mut::<AudioContext>() else {
        return;
    };
    for snapshot in &snapshots {
        apply_snapshot(ctx, snapshot);
    }
    let finished = ctx.prune_emitter_voices(&alive);

    for entity in finished {
        if let Some(spatial) = world.get_component_mut::<SpatialAudioEmitter>(entity) {
            spatial.emitter.playing = false;
        }
    }
}

/// Runs the action chosen for one emitter this frame.
fn apply_snapshot(ctx: &mut AudioContext, snapshot: &EmitterSnapshot) {
    let entity = snapshot.entity;
    let emitter = &snapshot.emitter;
    let sound_ready = emitter.sound_handle.is_some_and(|h| ctx.has_sound(h));
    let has_voice = ctx.has_emitter_voice(entity);
    match emitter_action(emitter.playing, sound_ready, has_voice) {
        EmitterAction::Idle => {}
        EmitterAction::Start => {
            if let Some(sound) = emitter.sound_handle {
                let volume = emitter_target_volume(ctx, entity, emitter);
                ctx.start_emitter_voice(
                    entity,
                    sound,
                    emitter.position,
                    emitter.max_distance,
                    volume,
                    emitter.pitch,
                    emitter.looping,
                );
            }
        }
        EmitterAction::Update => {
            let volume = emitter_target_volume(ctx, entity, emitter);
            ctx.update_emitter_voice(entity, emitter.position, volume);
        }
        EmitterAction::Stop => ctx.stop_emitter_voice(entity),
    }
}
