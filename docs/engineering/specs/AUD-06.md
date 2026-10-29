# AUD-06 — Адаптивная музыка: слои (стемы), громкость слоёв от игрового параметра

- **Размер:** M (три шага по S + живая проверка владельца). **Крейт:** `redox_audio`. **Зависит от:** AUD-01 (готово, музыка с диска). **Блокирует:** ничего.
- **Модель:** haiku.
- **Условие готовности (шаг 0, владелец):** нужны стемы — 2–4 файла Ogg одинаковой длины и темпа, начинающиеся в одной точке (например `explore_base.ogg`, `explore_tension.ogg`, `explore_combat.ogg`).
  Для автотестов хватает `tests/data/tone.ogg`, поэтому шаги 1–3 исполнитель делает без стемов; шаг 4 (слушать) — после появления ассетов. Положить стемы в каталог ассетов игры, не в `crates/`.

## Суть
Сейчас музыка — это «одна дорожка в каждый момент» с кроссфейдом (`MusicDirector`, `music.rs`; проигрывание — `music_play.rs`). Адаптивная музыка — это несколько стемов одной темы,
играющих **одновременно**, а игра плавно двигает громкость слоёв от числа 0..1 («напряжённость»): тихо — только база, враг рядом — подключается перкуссия, бой — всё.
Это самый дешёвый вид адаптивной музыки: никакой нарезки по тактам и переходов по маркерам.

## Что уже есть (проверено чтением)
- `MusicDirector` (`music.rs:29`): `play`, `stop`, `current`, `update(dt) -> MusicFrame { gains, finished }`; честный кроссфейд равной мощности `sin(level·π/2)`.
- `MusicVoice` (трейт, `music_apply.rs:12`: `set_volume(gain)`, `stop()`), `MusicVoices<V>` (карта имя→голос, `apply(&MusicFrame)`), `KiraMusicVoice` (`music_play.rs:45`).
- `AudioContext`: `register_music` / `register_music_file` (`music_play.rs:68,82`), `play_music`, `stop_music`, `update_music`, приватный `start_music_voice` (`music_play.rs:147`) и `start_music_stream` (`context.rs:317`; запускает **зацикленный тихий** поток на шине Music).
- Тесты музыки пропускают проверки с устройством через `ctx.is_available()` (`tests/music_streaming.rs:68–70`); логика проверяется без устройства через фейковые `MusicVoice` (`tests/music_apply.rs`).

## Модель данных (шаг 1: чистая логика, без kira)
Файл `src/music_layers.rs`:
```rust
/// One stem of an adaptive track and the range of the control value where it is audible.
#[derive(Clone, Debug, PartialEq)]
pub struct MusicLayer {
    /// Name of a registered music track (see `register_music`).
    pub track: String,
    /// The control value at which the layer starts to fade in (0..1).
    pub fade_in_from: f32,
    /// The control value at which the layer is fully in (>= fade_in_from).
    pub full_from: f32,
}

/// A set of layers driven by one control value. Layers that start at 0 are the always-on base.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct AdaptiveMusic { pub layers: Vec<MusicLayer> }

impl AdaptiveMusic {
    /// Linear gain of every layer (same order as `layers`) for `control` in 0..1 (clamped; NaN -> 0). A layer with
    /// `fade_in_from >= full_from` switches on at `full_from` (no division by zero). Output is in 0..=1.
    pub fn gains(&self, control: f32) -> Vec<f32>;
}
```
Формула: `t = clamp((control - fade_in_from) / (full_from - fade_in_from), 0, 1)`, гейн `sin(t·π/2)` (равная мощность, как в `MusicDirector`).
Сглаживание во времени (чтобы громкость не прыгала, когда игра меняет число скачком):
```rust
/// Moves `current` toward `target` by at most `rate * dt` (rate in gain units per second). NaN dt/rate leave `current` unchanged.
pub fn approach(current: f32, target: f32, rate: f32, dt: f32) -> f32;
```
Тесты (юнит, в файле):
```rust
#[test] fn the_base_layer_is_full_at_zero() { /* fade_in_from 0.0, full_from 0.0 -> gains(0.0)[0] == 1.0 */ }
#[test] fn a_layer_is_silent_below_its_range_and_full_above_it() { /* 0.3..0.6: gains(0.2)==0, gains(0.7)==1 */ }
#[test] fn a_layer_is_half_power_in_the_middle() { /* 0.0..1.0 at 0.5: (g*g - 0.5).abs() < 1e-6 ? sin(pi/4)^2 = 0.5 */ }
#[test] fn control_is_clamped_and_nan_is_zero() { /* gains(-5)==gains(0); gains(9)==gains(1); gains(NAN)==gains(0) */ }
#[test] fn equal_edges_switch_on_without_dividing_by_zero() { /* fade_in_from == full_from == 0.5 -> 0 below, 1 at/above, no NaN */ }
#[test] fn approach_does_not_overshoot_and_ignores_nan() { /* approach(0,1,2,0.25)==0.5; approach(0.9,1,2,1)==1; approach(0.3,1,NAN,1)==0.3 */ }
```

## Шаг 2 — проигрывание N слоёв (S, `music_play.rs`/новый `music_adaptive.rs`)
```rust
impl AudioContext {
    /// Starts every layer of `music` as a silent looping stream, all in the same call so they begin together.
    /// Layers whose track is not registered are skipped with a `log::warn!`. Replaces an adaptive track that is already playing.
    pub fn play_adaptive_music(&mut self, music: AdaptiveMusic);
    /// Sets the control value (0..1). The layer gains follow it smoothly (about 1.5 s from silence to full) on `update_music`.
    pub fn set_music_control(&mut self, control: f32);
    /// Stops the adaptive music with a fade.
    pub fn stop_adaptive_music(&mut self);
}
```
Реализация: хранить в `AudioContext` `adaptive: Option<AdaptiveState { music, control, current_gains: Vec<f32>, voices: Vec<Option<KiraMusicVoice>> }>`.
Голоса запускать через существующие `start_music_voice`-логику (вынести общий «открыть поток по имени» в приватную функцию; поведение `play_music` не менять).
`update_music(dt)` дополнить: если `adaptive` есть — `target = music.gains(control)`, `current[i] = approach(current[i], target[i], 1.0/1.5, dt)`, `voice.set_volume(current[i])`.
Не смешивать с кроссфейдом `MusicDirector`: при `play_music(..)` адаптивная музыка плавно гасится (простое правило, записать в doc-комментарий).
Синхронность старта: слои запускаются подряд в одном вызове на аудиопоток kira; **ПРОВЕРИТЬ:** хватает ли этого на слух. Если слои «плывут» — в отчёт, а решение (часы kira, `StartTime::ClockTime`) — отдельная задача.

Тесты шага 2 (без устройства — логика и отсутствие паники; с устройством — число голосов, `if !ctx.is_available() { return; }`):
```rust
#[test] fn adaptive_music_without_a_device_does_not_panic() { /* play_adaptive_music(two layers); set_music_control(0.7); update_music(0.1) */ }
#[test] fn an_unregistered_layer_is_skipped() { /* with a device: register one of two tracks; playing_music_voices() == 1 */ }
#[test] fn with_a_device_all_registered_layers_start_together() { /* register tone.ogg twice under two names; playing_music_voices() == 2 */ }
#[test] fn stopping_adaptive_music_removes_the_voices_after_the_fade() { /* with a device; stop; update_music(5.0); playing_music_voices() == 0 */ }
```

## Шаг 3 — подключение к игре (S)
Документация в `docs/` (раздел про звук, найти `grep -rn "play_music" docs | head`): пример `play_adaptive_music` + `set_music_control`. Реэкспорт `AdaptiveMusic`, `MusicLayer` в `lib.rs`.
Пример подключения к примеру игры не делать (игры вне этой спеки).

## Шаг 4 — живая проверка (владелец)
Со стемами: запустить, менять control от 0 до 1 и обратно; слышно ли, что слои входят плавно, не плывут относительно друг друга, нет щелчков. Результат записать в `PROGRESS.md`.

## Границы
- **Читать:** `music.rs`, `music_apply.rs`, `music_play.rs`, `context.rs:43–78,317–330`, `tests/music_apply.rs`, `tests/music_streaming.rs`.
- **Создать:** `src/music_layers.rs`, `src/music_adaptive.rs` (шаг 2), `tests/music_adaptive.rs`.
- **Менять:** `context.rs` (поле), `music_play.rs` (общая функция открытия потока, вызов из `update_music`), `lib.rs`.
- **Не трогать:** `MusicDirector` (`music.rs`), шины, `cues.rs`.

## Проверка
`cargo fmt`, `cargo clippy -p redox_audio --all-targets -- -D warnings`, `cargo test -p redox_audio`, `python scripts/check_file_sizes.py`, `python scripts/gen_crates_doc.py --check`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` скопировать из main. Все прежние музыкальные тесты зелёные без правок.
