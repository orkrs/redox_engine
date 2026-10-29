# NET-02 — Репликация ECS: снимки, дельты, интерполяция удалённых сущностей

- **Статус:** не начат.

- **Размер:** 5 шагов (S, M, M, M, S) ≈ 10 дней. **Крейты:** `redox_net`, `redox_core` (подключение), `redox_ecs`/`redox_reflect` (чтение полей). **Зависит от:** NET-01 (транспорт, Loopback, тики). **Блокирует:** NET-03.
- **Модель:** haiku по шагам 1, 5; шаги 2–4 — haiku с проверкой координатора.
- Объём: сервер владеет миром и раз в тик рассылает **изменения** реплицируемых сущностей; клиент создаёт/обновляет/удаляет их копии и **интерполирует** между снимками. **Не входит:** предсказание ввода и откат (NET-03), интерес-менеджмент по расстоянию (только простая «все всем» + лимит байт/тик), передача больших ресурсов, RPC.

## Суть
Реплицировать нужно ограниченный набор: позиция/поворот (`Transform`), скорость (для экстраполяции), состояние анимации (параметры графа — несколько f32), здоровье и произвольные игровые поля, помеченные игрой. Универсальной рефлексии компонентов в сети в v1 не строим: игра регистрирует **реплицируемые типы** явно (`app.replicate::<Transform>()`), через трейт.

## Шаг 1 — идентификаторы и описание реплицируемых типов (S, чистые типы)
```rust
/// Stable across machines; assigned by the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NetId(pub u32);
pub trait Replicated: Component + Clone + PartialEq {
    const TYPE_ID: u16;                       // уникален в проекте, проверяется реестром
    fn write(&self, w: &mut BitWriter);       // квантование внутри
    fn read(r: &mut BitReader) -> Result<Self, DecodeError>;
    fn interpolate(a: &Self, b: &Self, t: f32) -> Self { a.clone() }   // по умолчанию без интерполяции
}
pub struct ReplicationRegistry { /* TYPE_ID -> функции чтения/записи/применения */ }
```
- `BitWriter`/`BitReader` (посчитать биты, не байты): `write_bool`, `write_u(value, bits)`, `write_f32_quantized(v, min, max, bits)`, `write_quat_smallest_three`, `read_*`.
- Реализации для `Transform`: позиция 3×(f32 квантовано 20 бит на диапазон ±1024 м → шаг ≈ 2 мм), кватернион «smallest three» 3×10 бит + 2 бита индекса; `interpolate` = lerp/slerp.
- `NetId`-карта: `NetIdMap { by_net: HashMap<NetId, Entity>, by_entity: HashMap<Entity, NetId> }`.
Тесты (юнит):
```rust
#[test] fn bits_roundtrip_for_every_width() { /* 1..=32 бит */ }
#[test] fn quantized_floats_stay_within_the_step() { /* ±шаг/2 */ }
#[test] fn the_quaternion_roundtrips_within_a_small_angle() { /* ошибка < 0.1° на 10000 случайных кватернионах */ }
#[test] fn registering_the_same_type_id_twice_is_an_error() { /* ... */ }
#[test] fn a_transform_costs_at_most_12_bytes() { /* 3*20 + 32 = 92 бита -> 12 байт */ }
#[test] fn net_id_map_is_bijective() { /* вставка/удаление */ }
```

## Шаг 2 — серверные снимки и дельты (M)
- Серверная система `replication_send_system` (раз в тик): для каждой реплицируемой сущности (`Replicate` — маркер-компонент + `NetId`) собирает значения зарегистрированных компонентов; хранит **последний подтверждённый клиентом снимок** (клиент присылает `ack_tick` в каждом пакете ввода/пинга); шлёт для каждого клиента **дельту** относительно подтверждённого тика: список (NetId, mask изменённых типов, данные), удалённые NetId, созданные NetId с полным набором. Пакет ограничен `MAX_SNAPSHOT_BYTES = 1200` (одна дейтаграмма): при превышении — приоритет по давности последней отправки (накопление приоритета, «кто давно не получал»), остальное — в следующий тик.
- Канал `Snapshots` (ненадёжный, устаревшие отбрасываются). Создание/удаление дублируется в `Reliable` (иначе потеря удаления оставляет «призрака»): сервер хранит список событий жизненного цикла до подтверждения.
Тесты (Loopback):
```rust
#[test] fn an_unchanged_world_sends_an_empty_delta() { /* размер пакета ≤ заголовок */ }
#[test] fn only_changed_components_are_in_the_delta() { /* изменили x у одного -> 1 запись */ }
#[test] fn a_lost_snapshot_is_covered_by_the_next_delta_from_the_last_ack() { /* 50% потерь: клиент сходится */ }
#[test] fn the_packet_never_exceeds_the_limit_and_nothing_starves() { /* 500 сущностей: каждая обновлена ≥ раз в 10 тиков */ }
#[test] fn a_despawn_reaches_the_client_even_if_the_snapshot_is_lost() { /* через Reliable */ }
```

## Шаг 3 — клиентское применение и интерполяция (M)
- `replication_receive_system`: применяет снимки к `World` клиента: создаёт сущность на `Created`, обновляет компоненты, удаляет по `Despawned`; компонент `NetId` + `Remote` (маркер «не симулируется локально»).
- Буфер состояний для интерполяции: для каждой удалённой сущности хранится окно из последних ≤ 8 снимков `Transform` с серверным тиком; **время отображения** = `server_tick_estimate - INTERP_DELAY` (по умолчанию 3 тика = 100 мс при 30 Гц); `interpolate(a, b, t)` между ближайшими; если буфер пуст впереди (пропажа пакетов) — экстраполяция по скорости не дольше `MAX_EXTRAPOLATION = 0.25 с`, затем остановка.
- Сглаживание: без телепортации — если разность позиции между снимками > `TELEPORT_DISTANCE` (10 м), без интерполяции (флаг).
Тесты (Loopback, детерминированно):
```rust
#[test] fn a_remote_entity_appears_moves_and_disappears() { /* ... */ }
#[test] fn interpolation_is_exact_between_snapshots_with_constant_velocity() { /* позиция == линейная ±1 мм */ }
#[test] fn extrapolation_stops_after_the_limit() { /* ... */ }
#[test] fn a_teleport_does_not_slide_across_the_map() { /* ... */ }
#[test] fn jitter_in_arrival_times_does_not_make_motion_jerky() { /* джиттер ±15 мс: вторая разность позиции в допуске */ }
#[test] fn two_clients_converge_on_the_same_server_state() { /* ... */ }
```

## Шаг 4 — интеграция в `redox_core`, пример и замер полосы (M)
- `app.replicate::<T>()` в `NetPlugin`; `Replicate` маркер добавляется игрой; сервер назначает `NetId`.
- Пример `crates/redox_core/examples/net_arena.rs`: сервер двигает 100 кубов по детерминированной траектории, два клиента отображают (без окна — `--selftest`: сравнить позиции на клиентах с серверной на момент тика ± допуск; выход 0/1; с окном — обычный рендер).
- Бенчмарк `#[ignore]`: 100 и 500 сущностей, полоса вниз на клиента (байт/с) и CPU сервера (мкс/тик). Цель: 100 сущностей ≤ 30 КБ/с на клиента, CPU ≤ 0,3 мс/тик.
Тесты: `the_selftest_of_net_arena_passes_over_loopback`, `bandwidth_for_100_moving_entities_stays_within_target` (расчёт на Loopback по числу байт).

## Шаг 5 — документация (S)
`docs/NETWORKING.md` (как реплицировать компонент, ограничения, диаграмма потока «ввод → сервер → снимок → интерполяция»), `PLAN_TECH_OVERVIEW.md`, `PLAN_EXECUTION.md`, `CHANGELOG.md`.

## Границы
- **Читать:** NET-01, `redox_net/src/*`, `redox_ecs` (компоненты, `World`), `redox_reflect` (если нужен для имён), `redox_core/src/plugin.rs`, `redox_math` (кватернион).
- **Создать:** `redox_net/src/{bits,replicate,registry,snapshot,interp}.rs`, `redox_core/examples/net_arena.rs`, `tests/`, `docs/NETWORKING.md`.
- **Менять:** `redox_net/src/lib.rs`, `redox_core/src/plugin.rs`.
- **Не трогать:** транспорт (NET-01), физику.

## Проверка
`cargo fmt`, `cargo clippy -p redox_net -p redox_core --all-targets -- -D warnings`, `cargo test -p redox_net`, `cargo run -p redox_core --example net_arena --release -- --selftest`, `cargo check --workspace --all-targets`, `python scripts/check_file_sizes.py`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. NET-01 должен быть в `main` и ADR подтверждён; иначе остановись. Все тесты — на `LoopbackTransport`, без реальных сокетов.
