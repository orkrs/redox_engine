# PHY-06 — Контроллер персонажа: перенос вращения платформы и прилипание на крутых спусках

- **Статус (2026-10-30):** причина запаздывания ≈1 тик найдена и исправлена (перенос читал позу коллайдера, которую Rapier обновляет только внутри шага; теперь читается поза тела). Тест `a_platform_turn_is_carried_in_the_same_tick` написан, **не запускался** (cargo недоступен в этой сессии). Остаток: пропорциональная потеря ≈4–5 % поворота не объяснена; шаг 3 (PLAN_EXECUTION B5 ✅) не закрыт до измерения. **Известный дефект (до исправления):** при вращении платформы теряется ≈4–5 % поворота (дрейф 0,16 м за 1,5 с и 0,26 м за 3 с при радиусе 2 м и 1 рад/с; постоянное запаздывание ≈3 тика даёт ещё 0,12 м при вращении+переносе); окружность и радиус за 1 с в допуске. Тесты проверяют верхнюю границу, а не 5 см из спеки; причина потери не найдена — отдельная задача.

- **Размер:** 3 шага (S, S, S) ≈ 3 дня. **Крейт:** `redox_physics`. **Зависит от:** PHY-01…03 (готовы). **Блокирует:** — (NET-03 использует шаг контроллера).
- **Модель:** haiku (физические числа — стратегия «остановись и сообщи»).
- Закрывает «нет: перенос вращения платформы, спец-режимы» из `PLAN_EXECUTION.md` B5.

## Суть и аудит
- `character.rs`: `platform_carry(colliders, ground_pose) -> Vec3` (строка ~37) переносит персонажа линейной скоростью платформы; **вращение** платформы (карусель, поворотный мост) не учитывается — персонаж стоит на месте, пока платформа вращается под ним. Прилипание к земле (`snap_to_ground`) работает на склонах до `max_slope`; на спусках круче `snap_slope` (но ходимых) персонаж «отскакивает» короткими прыжками, а на ступенях при быстром спуске отрывается.
- Скорость платформы определяется как разность положения за шаг (`ground_pose` — `(ColliderHandle, Vec3)`; **ПРОВЕРИТЬ** что хранится и откуда).

## Шаг 1 — перенос вращения (S)
- В `ground_pose` добавить ориентацию платформы: `GroundPose { handle, position: Vec3, rotation: Quat }`; в `platform_carry` вычислять смещение точки опоры при вращении платформы за шаг: `delta = (rot_now * rot_prev⁻¹) * (p_char − pos_prev) + (pos_now − pos_prev) − (p_char − pos_prev)`; применять к желаемому смещению. Вращение персонажа вокруг вертикали вместе с платформой (`yaw_delta`) — **параметр** `CharacterSettings::inherit_platform_yaw: bool` (по умолчанию false; игра решает).
- Чистая функция `rotate_about(point, pivot, delta_rot) -> Vec3` и `yaw_of(quat) -> f32` с тестами.
Тесты (`redox_physics/tests/character_platforms.rs`, рядом с существующим тестом платформ):
```rust
#[test] fn a_character_on_a_rotating_platform_moves_along_the_circle() { /* платформа вращается 1 рад/с вокруг оси Y, персонаж стоит на радиусе 2 м: через 1 с угол позиции относительно оси ≈ 1 рад ±0.1, расстояние до оси 2 ±0.05 */ }
#[test] fn the_character_stays_at_the_same_point_of_the_platform_in_its_local_frame() { /* ошибка ≤ 5 см за 3 с */ }
#[test] fn a_platform_that_rotates_and_translates_carries_the_character_with_both() { /* ... */ }
#[test] fn inheriting_yaw_turns_the_character_with_the_platform() { /* флаг включён: yaw_delta ≈ угол платформы ±0.05 */ }
#[test] fn the_old_linear_platform_test_still_passes() { /* регресс */ }
```

## Шаг 2 — прилипание на крутых спусках (S)
- Диагностика сначала: тест-воспроизведение `walking_down_a_steep_slope_stays_grounded` (склон 40° при `max_slope = 50°`, ходьба вниз 3 с): доля тиков с `grounded == false` должна быть ≤ 5 %. Если на текущем коде красный — измерить число (находка), затем исправить: `snap_to_ground` дистанция = `max(snap_distance, step_len * tan(slope_angle_below) + eps)` (увеличивать по скорости спуска), ограничение — `max_snap`.
- Тесты:
```rust
#[test] fn walking_down_a_steep_slope_stays_grounded() { /* ≤ 5% тиков в воздухе */ }
#[test] fn a_ledge_still_lets_the_character_fall() { /* обрыв > max_snap: падает */ }
#[test] fn jumping_still_leaves_the_ground_on_a_slope() { /* прыжок отрывает: snap не мешает */ }
#[test] fn walking_down_stairs_keeps_contact() { /* лестница 20 см × 10: grounded ≥ 90% тиков */ }
#[test] fn the_snap_does_not_pull_the_character_up_a_wall() { /* ... */ }
```

## Шаг 3 — документация (S)
Строки `PLAN_EXECUTION.md` B5 (✅ полностью), `PROGRESS.md`, `CHANGELOG.md`; doc-комментарии новых полей `CharacterSettings`.

## Границы
- **Читать:** `redox_physics/src/{character,components,context,sync}.rs`, `tests/{character_v2,character_platforms}.rs`.
- **Создать:** тесты по необходимости.
- **Менять:** `character.rs`, `components.rs` (поле настроек).
- **Не трогать:** joints, контакты.

## Проверка
`cargo fmt`, `cargo clippy -p redox_physics --all-targets -- -D warnings`, `cargo test -p redox_physics`, `cargo check --workspace --all-targets`, `python scripts/check_file_sizes.py`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Если числа допусков недостижимы (вращение, прилипание) — остановись и сообщи измеренные значения. Не менять сигнатуры публичного API без нужды (поля настроек — только добавлять, с `Default`).

## Закрытие (финальная волна)

Исход обязателен: ✅ / ⛔ / 📌 (`../FINAL_WAVE.md` §1). Агенту разрешён `cargo check -p <крейт> --all-targets` в своём worktree; тесты, clippy и замеры — у координатора. Шаг, у которого приёмочный тест после доработки красный, не остаётся «частично»: агент сообщает измеренное число, исход назначает координатор.

- **Осталось:** диагностика остаточного дрейфа (half 0,161 м / full 0,260 м, потеря поворота 4–5 %). Порядок: (1) разложить потерю по компонентам: тангенциальная скорость платформы, дискретизация угла, усечение в `move_shape`; (2) проверить гипотезу KCC (отключить автоподъём и снап), записать числа; (3) причина найдена — исправить, цель потеря < 1 %; иначе 📌 в `KNOWN_LIMITS.md` с числами.
- **Приёмка:** тест платформы с поворотом проходит с допуском 1 % либо исход 📌. Отметить `PLAN_EXECUTION` B5 ✅ или 📌.
