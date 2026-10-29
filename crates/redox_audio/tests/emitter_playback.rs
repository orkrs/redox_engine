//! Playing `SpatialAudioEmitter`s (AUD-05): one kira spatial track per playing emitter. Tests that need a
//! sound card are skipped when there is none; the rest check that the system is harmless without one.

#![allow(clippy::unwrap_used)]

use redox_asset::{AssetId, Handle};
use redox_audio::{
    AudioContext, AudioData, AudioListener, SpatialAudioEmitter, emitter_playback_system,
    emitter_target_volume,
};
use redox_ecs::{Entity, World};
use redox_math::Vec3;

const TONE: &[u8] = include_bytes!("data/tone.ogg");

fn world() -> World {
    let mut world = World::new();
    world.insert_resource(AudioContext::new());
    let listener = world.spawn();
    world.add_component(listener, AudioListener::default());
    world
}

fn playing_emitter(world: &mut World, sound: Option<Handle<AudioData>>) -> Entity {
    let e = world.spawn();
    let mut emitter = SpatialAudioEmitter::new(Vec3::new(0.0, 0.0, -5.0));
    emitter.emitter.sound_handle = sound;
    emitter.emitter.looping = true;
    emitter.emitter.play();
    world.add_component(e, emitter);
    e
}

fn has_voice(world: &World, e: Entity) -> bool {
    world
        .get_resource::<AudioContext>()
        .unwrap()
        .has_emitter_voice(e)
}

fn available(world: &World) -> bool {
    world.get_resource::<AudioContext>().unwrap().is_available()
}

/// Decodes `tone.ogg` into the context and returns its handle.
fn load_tone(world: &mut World) -> Handle<AudioData> {
    let handle = Handle::<AudioData>::new(AssetId::next());
    let ctx = world.get_resource_mut::<AudioContext>().unwrap();
    assert!(ctx.add_sound_from_asset(handle, TONE));
    handle
}

#[test]
fn without_a_device_the_system_runs_and_changes_nothing_harmful() {
    let mut world = world();
    let e = playing_emitter(&mut world, None);
    emitter_playback_system(&mut world);
    emitter_playback_system(&mut world);
    assert!(!has_voice(&world, e));
}

#[test]
fn an_emitter_without_a_cached_sound_gets_no_voice() {
    let mut world = world();
    let unknown = Handle::<AudioData>::new(AssetId::next());
    let e = playing_emitter(&mut world, Some(unknown));
    emitter_playback_system(&mut world);
    assert!(!has_voice(&world, e));
    let emitter = world.get_component::<SpatialAudioEmitter>(e).unwrap();
    assert!(emitter.emitter.playing, "waiting for the sound keeps `playing`");
}

#[test]
fn with_a_device_a_playing_emitter_gets_a_voice_and_losing_playing_frees_it() {
    let mut world = world();
    if !available(&world) {
        return;
    }
    let tone = load_tone(&mut world);
    let e = playing_emitter(&mut world, Some(tone));
    emitter_playback_system(&mut world);
    assert!(has_voice(&world, e));
    world
        .get_component_mut::<SpatialAudioEmitter>(e)
        .unwrap()
        .emitter
        .stop();
    emitter_playback_system(&mut world);
    assert!(!has_voice(&world, e));
}

#[test]
fn with_a_device_a_despawned_emitter_loses_its_voice() {
    let mut world = world();
    if !available(&world) {
        return;
    }
    let tone = load_tone(&mut world);
    let e = playing_emitter(&mut world, Some(tone));
    emitter_playback_system(&mut world);
    assert!(has_voice(&world, e));
    world.despawn(e);
    emitter_playback_system(&mut world);
    assert!(!has_voice(&world, e));
}

#[test]
fn an_occluded_emitter_is_quieter_than_a_clear_one() {
    let mut world = world();
    let blocked = playing_emitter(&mut world, None);
    let clear = playing_emitter(&mut world, None);
    world
        .get_resource_mut::<AudioContext>()
        .unwrap()
        .set_emitter_occlusion(blocked, 0.8);
    let ctx = world.get_resource::<AudioContext>().unwrap();
    let volume = |e: Entity| {
        let emitter = &world.get_component::<SpatialAudioEmitter>(e).unwrap().emitter;
        emitter_target_volume(ctx, e, emitter)
    };
    assert!(volume(blocked) < volume(clear));
    assert!((volume(clear) - 1.0).abs() < 1e-6);
}
