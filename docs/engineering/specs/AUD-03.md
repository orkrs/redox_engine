# AUD-03 — Окклюзия звука: по источнику, а не глушение всего; триггеры не стены

- **Статус:** выполнено 2026-10-29, принято в `main`. Допущение спецификации было неверным: `update_spatial_parameters` нигде не вызывается и систем, проигрывающих `SpatialAudioEmitter`, нет. Глобальное глушение убрано, поэтому окклюзия пока не слышна → AUD-05. Значения: 0 / 0,8 / 0,92 / 0,97.

- **Размер:** S (две части по одному файлу-крейту). **Крейты:** `redox_physics` (часть A), `redox_audio` (часть B). **Зависит от:** ничего.
- **Модель:** haiku (часть A — одна строка + готовый тест; часть B — три небольших изменения и тесты).
- **Заменяет** прежний пункт «окклюзия звука по физике» из плана: сама окклюзия по лучам **уже есть**
  (`redox_audio/src/systems.rs::occlusion_raycast_system`, подключена в `redox_core/src/plugin.rs:87`), но работает неверно.

## Что найдено в коде (проверено чтением и запуском)
1. `PhysicsContext::cast_ray_excluding` (`redox_physics/src/context.rs:435`) использует `QueryFilter::default()` и **считает преградой сенсоры** (триггеры).
   Тест `redox_physics/tests/ray_sensors.rs::a_trigger_volume_does_not_block_the_ray` с `#[ignore]` падает: луч через триггерный шар попал в него на расстоянии 4,0.
   Любой триггер между слушателем и источником заглушает звук.
2. `occlusion_raycast_system` в конце вызывает `audio_ctx.apply_occlusion_lowpass(max_occ)` (`systems.rs`, около строки 180): один закрытый источник
   включает ФНЧ на **главной дорожке** — глушатся музыка, интерфейс и все остальные звуки. А поштучная окклюзия (`set_emitter_occlusion`,
   `get_emitter_occlusion`, `volume_multiplier_from_occlusion`) нигде не применяется к громкости.
3. Преграда одна: попадание → 0,8 всегда, толщина и число стен не учитываются.

## Границы
- **Читать:** `redox_physics/src/context.rs` (строки 399–470: `cast_ray_with_normal`, `cast_ray_excluding`), `redox_physics/tests/ray_sensors.rs`;
  `redox_audio/src/systems.rs` (строки 36–190), `context.rs` (строки 360–460: `apply_occlusion_lowpass`, `*_emitter_occlusion`, `update_spatial_parameters`), `components.rs` (`SpatialAudioEmitter`).
- **Менять:** `redox_physics/src/context.rs`, `redox_physics/tests/ray_sensors.rs` (снять `#[ignore]`); `redox_audio/src/systems.rs`, `redox_audio/src/context.rs`, новый `redox_audio/tests/occlusion.rs`.
- **Не трогать:** `redox_core`, шины и миксер, реверберацию.

## Решение
### Часть A (physics)
В `cast_ray_excluding` и (по той же причине) `cast_ray_with_normal`: `QueryFilter::default().exclude_sensors().predicate(&predicate)`
(метод `exclude_sensors` у `QueryFilter` rapier; если он называется иначе в 0.36 — искать `QueryFilterFlags::EXCLUDE_SENSORS`). Для `cast_ray_with_normal` сначала проверить,
не нужна ли пользователям (игровой код, `raycast_system`) возможность попадать в сенсоры: тесты `tests/queries.rs` и `tests/layers_in_ecs.rs` должны остаться зелёными;
если ломаются — менять **только** `cast_ray_excluding`, а в отчёте написать про вторую функцию.

### Часть B (audio)
1. Чистая функция в `systems.rs` (публичная, для тестов):
   ```rust
   /// Occlusion from the number of solid hits between listener and emitter: 0 hits -> 0.0, then 0.5, 0.8, 0.95 (max).
   pub fn occlusion_from_hits(hits: usize) -> f32;
   ```
   Таблица: `0 → 0.0`, `1 → 0.8` (сохраняет прежнее поведение для одной стены), `2 → 0.92`, `≥3 → 0.97`.
2. В `occlusion_raycast_system` вместо одного луча — цикл: после попадания в `entity` добавить его в `exclude` и повторить луч (максимум 3 итерации, чтобы запрос
   был ограничен). Результат: `occlusion_from_hits(hits)`.
3. Убрать из системы вызов `apply_occlusion_lowpass(max_occ)` (глобальное глушение). Сам метод оставить (его можно использовать явно, например для подводного режима).
4. Громкость: в `AudioContext::update_spatial_parameters(...)` (`context.rs:432`) принять множитель окклюзии: громкость умножается на
   `volume_multiplier_from_occlusion(get_emitter_occlusion(entity))`. Если у метода нет параметра `entity`, добавить новый метод
   `fn emitter_volume(&self, entity: Entity, base: f32) -> f32 { base * Self::volume_multiplier_from_occlusion(self.get_emitter_occlusion(entity)) }` и вызывать его из мест, где
   считается громкость эмиттера (поиск: `grep -rn "update_spatial_parameters" crates`). Старую сигнатуру **не ломать**.
5. Ограничение (записать в doc-комментарий системы): ФНЧ на источник не делается — для этого нужна отдельная дорожка kira на эмиттер; это отдельная задача и в план не входит.

## Тесты
Часть A: тест `a_trigger_volume_does_not_block_the_ray` уже написан — снять `#[ignore]`.

Часть B (`redox_audio/tests/occlusion.rs`; `AudioContext::new()` создаётся и без устройства — так сделано в `music_context.rs`; физика — как в `redox_physics/tests/queries.rs`):
```rust
use redox_audio::systems::occlusion_from_hits;   // проверить путь экспорта; при необходимости реэкспортировать в lib.rs

#[test] fn hits_map_to_the_documented_occlusion() {
    assert_eq!(occlusion_from_hits(0), 0.0);
    assert_eq!(occlusion_from_hits(1), 0.8);
    assert!(occlusion_from_hits(2) > occlusion_from_hits(1));
    assert_eq!(occlusion_from_hits(3), occlusion_from_hits(10));
}
#[test] fn a_clear_line_leaves_the_emitter_untouched() {
    // world: PhysicsContext (no colliders), AudioListener at origin, SpatialAudioEmitter at z = -10, AudioContext resource.
    // occlusion_raycast_system(&mut world); emitter.occlusion_coefficient == 0.0; ctx.get_emitter_occlusion(e) == 0.0
}
#[test] fn two_walls_occlude_more_than_one() {
    // fixed boxes at z=-3 and z=-6, emitter at z=-10 -> coefficient == occlusion_from_hits(2); with one box -> occlusion_from_hits(1)
}
#[test] fn one_blocked_emitter_does_not_muffle_the_rest() {
    // two emitters, one behind a wall, one in the clear; after the system the clear one has coefficient 0.0
    // and emitter_volume(clear, 1.0) == 1.0, emitter_volume(blocked, 1.0) < 1.0.
    // The main-track filter is not touched: add a spy — AudioContext::lowpass_cutoff_hz() -> Option<f32> is NOT available,
    // so check the behaviour instead: the system must not call apply_occlusion_lowpass (grep in the test is not allowed) —
    // verified in review of the diff.
}
#[test] fn a_trigger_between_listener_and_emitter_does_not_occlude() {
    // add_trigger ball between them -> coefficient 0.0 (needs part A)
}
```
Для тестов с физикой не забыть `ctx.step(1.0 / 60.0)` после добавления коллайдеров.

## Проверка
`cargo test -p redox_physics`, `cargo test -p redox_audio`, `cargo clippy -p redox_physics -p redox_audio --all-targets -- -D warnings`, fmt.
Живая проверка (владелец): ходить за стеной от источника в Courier/Foxfire — слышно приглушение громкости **только** этого источника.

## Для исполнителя
`git merge --ff-only main`. Сначала часть A (красный → зелёный `ray_sensors`), затем B. Если в `redox_audio` нет доступа к `PhysicsContext` в тесте
без лишней зависимости (dev-dependency) — она уже есть (система использует `redox_physics`), проверь `Cargo.toml`.
