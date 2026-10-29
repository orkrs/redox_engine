# AUD-05 — Проигрывание `SpatialAudioEmitter`: дорожка kira на сущность, громкость с окклюзией

- **Статус:** выполнено 2026-10-29 (код агента Haiku, тесты координатора).

- **Размер:** M (три шага по S). **Крейты:** `redox_audio` (шаги 1–2), `redox_core` (шаг 3, одна строка). **Зависит от:** AUD-03 (готово). **Блокирует:** ничего.
- **Модель:** haiku. Шаги делать по порядку, каждый — отдельный коммит.

## Суть
AUD-03 посчитала окклюзию для каждого эмиттера (`AudioContext::emitter_volume`, `occlusion_coefficient`), но **никто не проигрывает** `SpatialAudioEmitter`:
`update_spatial_parameters` нигде не вызывается, `AudioEmitter::playing` никто не читает (проверено `grep`: поле `playing` используется только в `components.rs` и тестах).
Игра играет звук через `play_spatial*` — это одноразовый вызов без дальнейшего управления, окклюзия к нему не применяется. Итог: окклюзия посчитана, но не слышна.

Нужна система: сущности с `SpatialAudioEmitter`, у которых `emitter.playing == true` и есть `sound_handle`, получают собственную пространственную дорожку kira;
позиция и громкость обновляются каждый кадр; при `playing == false`, смерти сущности или окончании звука дорожка освобождается.

## Что уже есть (проверено чтением)
- `AudioContext` (`context.rs:43`): поля `manager: Option<AudioManager>`, `listener: Option<ListenerHandle>`, `sound_cache: HashMap<AssetId, StaticSoundData>`, `occlusion_by_emitter`.
  Без устройства `manager == None`, все методы воспроизведения возвращают `None` — тесты должны это переживать (`is_available()`, `context.rs:194`).
- `play_spatial` (`context.rs:334`) строит дорожку так: `manager.add_spatial_sub_track(listener, position, SpatialTrackBuilder::new().distances((0.1, max)).persist_until_sounds_finish(true))`, затем `track.play(data)`. Дорожку не хранит.
  Хэндл `SpatialTrackHandle` (kira 0.12.5, `track/sub/spatial_handle.rs`) умеет `set_volume(impl Into<Value<Decibels>>, Tween)`, `set_position(..., Tween)`, `num_sounds()`, `state()`.
- `gain_to_db(f32) -> Decibels` (`context.rs:548`). `emitter_volume(entity, base)` (`context.rs:433`). `has_sound(handle)` (`context.rs:486`).
- Параметры зацикливания и высоты: как в `play_asset_id` (`context.rs:289–313`): `StaticSoundSettings::new().volume(..).playback_rate(..)`, цикл — `.loop_region(0.0..)`, затем `data.settings = settings`.
- Система окклюзии подключена в `redox_core/src/plugin.rs:86–90` (`Stage::PostUpdate`, порядок: окклюзия → слушатель → реверберация).
- Итерация по сущностям в крейте: `world.all_entities()` + `world.get_component::<T>(e)` (так сделано в `systems.rs:158–176`). `World::despawn` существует (`redox_ecs/src/world.rs:417`).
  Проверка «сущность ещё жива»: **ПРОВЕРИТЬ:** есть ли `World::contains`/аналог (`grep -n "pub fn" crates/redox_ecs/src/world.rs`). Если нет — собирать множество живых эмиттеров за кадр и удалять голоса, которых в нём нет (так и написано ниже, дополнительный API не нужен).

## Границы
- **Читать:** `context.rs` (целиком, 673 строки), `components.rs:8–60,133–190`, `systems.rs:150–225`, `lib.rs`, `redox_core/src/plugin.rs:78–125`, `tests/occlusion.rs`, `tests/music_streaming.rs` (как пропускать тест без устройства).
- **Создать:** `crates/redox_audio/src/emitter_voices.rs`, `crates/redox_audio/tests/emitter_playback.rs`.
- **Менять:** `context.rs` (поле `emitter_voices` и три метода), `lib.rs` (`mod emitter_voices; pub use emitter_voices::emitter_playback_system;`), `redox_core/src/plugin.rs` (одна строка в системе `PostUpdate`).
- **Не трогать:** `mixer.rs`, `cues.rs`, `music*.rs`, `systems.rs` (окклюзию), `components.rs` (ничего нового в компонентах не нужно).

## Шаг 1 — чистая логика: что делать с каждым эмиттером (S, без звукового устройства)
Файл `emitter_voices.rs`. Решение вынесено в чистую функцию, чтобы тестировать её без kira:
```rust
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

/// `has_voice`: the context already holds a voice for this entity. `sound_ready`: the emitter has a `sound_handle`
/// and `AudioContext::has_sound` is true for it.
pub(crate) fn emitter_action(playing: bool, sound_ready: bool, has_voice: bool) -> EmitterAction {
    match (playing && sound_ready, has_voice) {
        (true, false) => EmitterAction::Start,
        (true, true) => EmitterAction::Update,
        (false, true) => EmitterAction::Stop,
        (false, false) => EmitterAction::Idle,
    }
}

/// Linear volume of an emitter: its own `volume` (clamped to 0..) times the occlusion multiplier. Delegates to `AudioContext::emitter_volume`.
```
Громкость считать методом `AudioContext::emitter_volume(entity, emitter.volume.max(0.0))`, новой формулы не вводить.

Тесты (юнит, в том же файле, `#[cfg(test)]`):
```rust
#[test] fn an_emitter_that_plays_without_a_voice_starts() { assert_eq!(emitter_action(true, true, false), EmitterAction::Start); }
#[test] fn a_playing_emitter_with_a_voice_is_updated() { assert_eq!(emitter_action(true, true, true), EmitterAction::Update); }
#[test] fn a_stopped_emitter_with_a_voice_is_stopped() { assert_eq!(emitter_action(false, true, true), EmitterAction::Stop); }
#[test] fn losing_the_sound_stops_the_voice() { assert_eq!(emitter_action(true, false, true), EmitterAction::Stop); }
#[test] fn an_idle_emitter_without_a_voice_does_nothing() { assert_eq!(emitter_action(false, false, false), EmitterAction::Idle); }
#[test] fn a_playing_emitter_whose_sound_is_not_ready_waits() { assert_eq!(emitter_action(true, false, false), EmitterAction::Idle); }
```

## Шаг 2 — голоса в `AudioContext` и система (M)
В `context.rs`:
```rust
/// A spatial kira track playing one emitter's sound.
pub(crate) struct EmitterVoice {
    track: SpatialTrackHandle,      // use kira::track::SpatialTrackHandle (проверить путь импорта в kira 0.12.5)
    sound: StaticSoundHandle,
}
// поле AudioContext:
emitter_voices: HashMap<Entity, EmitterVoice>,
```
Методы (все `pub(crate)`, если не сказано иное; без устройства — возвращают `false`/ничего и не паникуют):
```rust
/// Starts a spatial voice for `entity`. False without a device, listener or cached sound.
pub(crate) fn start_emitter_voice(&mut self, entity: Entity, sound: Handle<AudioData>, position: Vec3, max_distance: f32,
                                  volume: f32, pitch: f32, looping: bool) -> bool;
/// Moves the voice and sets its volume (linear). No-op when there is no voice.
pub(crate) fn update_emitter_voice(&mut self, entity: Entity, position: Vec3, volume: f32);
/// Stops (short fade) and drops the voice of `entity`.
pub(crate) fn stop_emitter_voice(&mut self, entity: Entity);
/// True when `entity` has a voice.
pub fn has_emitter_voice(&self, entity: Entity) -> bool;      // pub: нужен тестам и отладке
/// Drops voices of entities not in `alive` and voices whose sound finished (non-looping). Returns the entities whose sound finished.
pub(crate) fn prune_emitter_voices(&mut self, alive: &std::collections::HashSet<Entity>) -> Vec<Entity>;
```
Тело `start_emitter_voice`: как `play_spatial` (`context.rs:334`), но `SpatialTrackBuilder::new().distances((0.1, max_distance.max(0.11)))` **без** `persist_until_sounds_finish`
(дорожка живёт, пока жив хэндл), громкость дорожки не трогать, громкость звука `gain_to_db(volume)`, высота `playback_rate(f64::from(pitch.max(0.01)))`, цикл `loop_region(0.0..)`.
Хэндл звука сохранить: `let sound = track.play(data).ok()?;` и положить `EmitterVoice` в карту.
`update_emitter_voice`: `track.set_position(position, Tween::default())` и `track.set_volume(gain_to_db(volume), Tween::default())`.
`stop_emitter_voice`: `sound.stop(Tween { duration: Duration::from_millis(30), ..Default::default() })`, затем `remove`.
Окончание звука: `sound.state() == kira::sound::PlaybackState::Stopped` (**ПРОВЕРИТЬ:** имя перечисления и метода в `kira-0.12.5/src/sound/static_sound/handle.rs`) → голос в `prune_emitter_voices` удаляется, эмиттеру выставляется `playing = false`.

Система в `emitter_voices.rs`:
```rust
/// Plays `SpatialAudioEmitter`s: one kira spatial track per playing emitter, position and volume (with occlusion) refreshed every frame.
/// Run after `occlusion_raycast_system`.
pub fn emitter_playback_system(world: &mut World);
```
Алгоритм: (1) собрать `Vec<(Entity, Vec3 position, f32 volume, f32 pitch, bool looping, bool playing, Option<Handle<AudioData>>, f32 max_distance)>` по `SpatialAudioEmitter`;
(2) без `AudioContext` в мире — выход; (3) для каждого: `sound_ready = handle.is_some_and(|h| ctx.has_sound(h))`, `action = emitter_action(..)`, исполнить `Start/Update/Stop`;
(4) `prune_emitter_voices(&alive)` и для вернувшихся сущностей `emitter.playing = false`.
Громкость: `ctx.emitter_volume(entity, volume.max(0.0))`.
Файл не более 250 строк; `context.rs` после правки обязан остаться < 800 строк (сейчас 673; если превысит — вынести методы голосов в `emitter_voices.rs` как `impl AudioContext`, поле сделать `pub(crate)`).

## Шаг 3 — подключение (S)
`redox_core/src/plugin.rs:86–90`: после `occlusion_raycast_system(world);` добавить `redox_audio::emitter_playback_system(world);` (порядок: окклюзия → голоса → слушатель). Комментарий над `AudioPlugin` обновить.

## Тесты (`tests/emitter_playback.rs`; без устройства система не должна паниковать)
```rust
#[test] fn without_a_device_the_system_runs_and_changes_nothing_harmful() {
    // world: AudioContext::new() (device may be absent), listener, one SpatialAudioEmitter { playing: true, sound_handle: None }.
    // emitter_playback_system(&mut world) twice; no panic; !ctx.has_emitter_voice(e).
}
#[test] fn an_emitter_without_a_cached_sound_gets_no_voice() { /* sound_handle: Some(Handle::new(..)) not registered -> no voice, playing stays true */ }
#[test] fn with_a_device_a_playing_emitter_gets_a_voice_and_losing_playing_frees_it() {
    // skip when !ctx.is_available(); sync tone.ogg via add_sound_from_asset(handle, bytes); looping emitter; run system -> has_emitter_voice;
    // set playing = false; run -> !has_emitter_voice.
}
#[test] fn with_a_device_a_despawned_emitter_loses_its_voice() { /* skip without device; despawn; run; !has_emitter_voice */ }
#[test] fn an_occluded_emitter_is_quieter_than_a_clear_one() {
    // логика без устройства: set_emitter_occlusion(e1, 0.8); assert!(ctx.emitter_volume(e1, 1.0) < ctx.emitter_volume(e2, 1.0)) — уже покрыто AUD-03,
    // здесь проверить, что система берёт именно emitter_volume: вынести расчёт в fn emitter_target_volume(ctx, e, emitter) и вызвать её напрямую.
}
```
Как получить `Handle<AudioData>` в тесте: **ПРОВЕРИТЬ:** `grep -rn "Handle::" crates/redox_audio/tests crates/redox_asset/src | head` (есть ли конструктор для тестов); данные — `include_bytes!("data/tone.ogg")`.

## Проверка
`cargo fmt`, `cargo clippy -p redox_audio -p redox_core --all-targets -- -D warnings`, `cargo test -p redox_audio`, `cargo test -p redox_core`, `python scripts/check_file_sizes.py`, `python scripts/gen_crates_doc.py --check`.
Живая проверка (владелец, QA-01): эмиттер с зацикленным звуком, ходить за стеной — слышно приглушение только этого источника.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` скопировать из main. Не менять поведение `play_spatial*` и одноразовых звуков. Чего не удалось проверить — в отчёт.
