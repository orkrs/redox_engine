# NET-03 — Предсказание ввода и согласование (локальный игрок без задержки)

- **Статус:** не начат.

- **Размер:** 4 шага (S, M, M, S) ≈ 8 дней. **Крейты:** `redox_net`, `redox_core`, `redox_physics` (детерминированный шаг контроллера). **Зависит от:** NET-02, PHY-01…03 (контроллер персонажа). **Блокирует:** —.
- **Модель:** haiku по шагам 1, 4; шаги 2–3 — haiku с проверкой координатора (тонкая логика: при расхождении с ожиданием — остановка и отчёт).
- Объём: **только персонаж-контроллер** (`CharacterController`), предсказание ввода на клиенте + откат/перемотка по ответу сервера. **Не входит:** предсказание произвольной физики (`rapier` тела), компенсация лагов для стрельбы (rewind сервера), восстановление после длительных (> 1 с) разрывов (клиент переподключается).

## Суть
Клиент отправляет ввод каждый тик (30 Гц), **немедленно** применяет его к своему персонажу (предсказание), хранит историю `(tick, input, predicted_state)`. Сервер симулирует тот же ввод авторитетно и в снимке присылает `(last_processed_input_tick, state)`. Клиент сверяет: если состояние на тик совпало в допуске — выбрасывает историю до этого тика; если нет — откатывает персонажа в серверное состояние и **пере-симулирует** оставшиеся вводы.
Требование: шаг контроллера — **детерминированная функция** `step(state, input, dt, world) -> state` (на сервере и клиенте одинаковая, фиксированный `dt = 1/30`).

## Что уже есть (проверено чтением)
- Контроллер: `redox_physics/src/character.rs` (`CharacterSettings`, `jump()`, ступени, склоны, платформы); использует rapier `KinematicCharacterController` с запросами к миру. Шаг привязан к `PhysicsContext`. Детерминизм между машинами **не гарантирован** (rapier без `enhanced-determinism`; float-порядок), но при одинаковой сборке и платформе (только Windows x64) даёт одинаковый результат для одинаковых входов — достаточный базис для v1 (записать как допущение в ADR 0008).

## Шаг 1 — чистая логика предсказания (S, без физики)
`redox_net/src/predict.rs` (обобщённая по типам `State`, `Input`):
```rust
pub struct PredictionBuffer<S, I> { /* кольцевой буфер ≤ 128 тиков */ }
impl<S: Clone, I: Clone> PredictionBuffer<S, I> {
    pub fn push(&mut self, tick: u32, input: I, state_after: S);
    /// Server says: state after processing input `tick` is `server_state`. Returns inputs to replay (ticks > `tick`) if the predicted state differs by more than `eq`, else `None`.
    pub fn reconcile(&mut self, tick: u32, server_state: &S, eq: impl Fn(&S, &S) -> bool) -> Option<Vec<(u32, I)>>;
    pub fn unacked(&self) -> usize;
}
pub fn smooth_correction(visual: Vec3, target: Vec3, error_distance: f32, dt: f32) -> Vec3;   // сглаживание визуальной ошибки: до 0.5 м — экспоненциально за ~100 мс; больше — мгновенно
```
Тесты:
```rust
#[test] fn matching_server_state_drops_the_acknowledged_history() { /* unacked уменьшается */ }
#[test] fn a_mismatch_returns_the_inputs_after_the_acknowledged_tick() { /* ... */ }
#[test] fn the_buffer_overwrites_the_oldest_entry_when_full() { /* ... */ }
#[test] fn an_ack_for_an_unknown_tick_is_ignored() { /* ... */ }
#[test] fn small_errors_are_smoothed_and_large_ones_snap() { /* ... */ }
#[test] fn replaying_inputs_from_the_server_state_reproduces_the_prediction_without_error() { /* игра на модели «x += input» */ }
```

## Шаг 2 — детерминированный шаг контроллера (M, `redox_physics`)
- Выделить из `character.rs` чистый шаг: `CharacterState { position, velocity_y, grounded, platform_velocity… }` + `CharacterInput { move_dir: [i8;2], jump: bool, tick: u32 }` (ввод квантован до целых — одинаково на двух машинах), `fn step_character(ctx: &mut PhysicsContext, id, state: &CharacterState, input: &CharacterInput, dt: f32) -> CharacterState`, не читающая время и случайные числа. Существующий публичный API остаётся (он вызывает новый шаг).
- Состояние сериализуется (`Replicated`, шаг NET-02): позиция 3×20 бит, скорость Y 16 бит, флаги.
Тесты (`redox_physics/tests/character_step.rs`): `the_same_inputs_produce_the_same_states_twice` (два контекста, 600 тиков, побитное равенство), `the_step_does_not_depend_on_wall_clock` (между вызовами `sleep` — результат тот же), `the_old_api_matches_the_new_step` (регресс `character_v2.rs` зелёный без правок), `a_jump_input_produces_the_same_arc_each_time`.

## Шаг 3 — клиент: предсказание, сверка, повтор; сервер: авторитет (M, `redox_net` + `redox_core`)
- Клиент: каждый тик — собрать ввод → `step_character` для локального игрока → `PredictionBuffer.push` → отправить `InputPacket { tick, inputs: последние 3 (избыточность против потерь) }` по каналу `Unreliable`.
- Сервер: очередь вводов клиента (по тику), применяет ввод `tick` в свой тик (если ввода нет — повторяет предыдущий не более 3 тиков, затем стоит), результат `CharacterState` + `last_input_tick` уходит в снимке (NET-02, компонент `PredictionAck`).
- Клиент при получении снимка: `reconcile(last_input_tick, server_state)`; при расхождении — откат (`position`, `velocity_y`, `grounded` из серверного состояния), повтор вводов `step_character` для хвоста; визуальный сдвиг сглаживается `smooth_correction`.
- Удалённые игроки — интерполяция (NET-02), **не** предсказание.
Тесты (интеграционные, Loopback, без окна; сервер и клиент в одном процессе с двумя `PhysicsContext`; задержка 60 мс, джиттер 10 мс, потери 5%):
```rust
#[test] fn the_local_player_reacts_to_input_in_the_same_frame() { /* позиция меняется в тике нажатия, до прихода ответа */ }
#[test] fn the_client_and_the_server_agree_after_a_run() { /* 20 с случайного (детерминированного) ввода: |client - server| < 2 см в конце */ }
#[test] fn a_server_side_push_is_reconciled_without_a_visible_snap_over_the_threshold() { /* сервер сдвинул игрока на 0.3 м: клиент сходится, визуальная скорость коррекции ≤ лимит */ }
#[test] fn lost_input_packets_are_covered_by_redundancy() { /* 20% потерь, 3 последних ввода в пакете: расхождение < 5 см */ }
#[test] fn the_server_ignores_inputs_from_the_future_and_the_distant_past() { /* tick+100 / tick-100 */ }
#[test] fn a_cheating_speed_input_is_clamped_by_the_server() { /* move_dir вне [-1,1] -> нормализуется; скорость ≤ max */ }
#[test] fn standing_on_a_moving_platform_stays_in_sync() { /* ... */ }
```

## Шаг 4 — пример и документация (S)
`net_arena` (NET-02) получает управляемого персонажа (WASD) у клиентов и ботов-ввода в `--selftest`; бенчмарк `#[ignore]`: ошибка предсказания (среднее/макс) при задержке 0/60/150 мс и потерях 0/5/15 %. Документы: `docs/NETWORKING.md` (раздел «Предсказание»), `docs/adr/0008-networking.md` (допущение про детерминизм), `PLAN_EXECUTION.md`, `CHANGELOG.md`.

## Границы
- **Читать:** `redox_physics/src/{character,context,sync}.rs`, `tests/character_v2.rs`, NET-01/02, `redox_core/src/plugin.rs`.
- **Создать:** `redox_net/src/predict.rs`, `redox_physics/tests/character_step.rs`, `redox_net/tests/prediction.rs`.
- **Менять:** `redox_physics/src/character.rs` (выделение шага без смены поведения), `redox_net/src/lib.rs`, `redox_core/src/*` (клиентская/серверная системы), `redox_core/examples/net_arena.rs`.
- **Не трогать:** остальную физику, транспорт.

## Проверка
`cargo fmt`, `cargo clippy -p redox_net -p redox_physics -p redox_core --all-targets -- -D warnings`, `cargo test -p redox_net --test prediction`, `cargo test -p redox_physics --test character_step --test character_v2`, `cargo run -p redox_core --example net_arena --release -- --selftest`, `cargo check --workspace --all-targets`, `python scripts/check_file_sizes.py`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. NET-02 в `main` обязателен. Если «клиент и сервер расходятся» в тесте шага 3 больше допуска из-за недетерминизма rapier — **остановись и сообщи измеренное расхождение** (решение за координатором: `enhanced-determinism`, фиксированный порядок тел или серверная авторитетность без предсказания для этого случая).
