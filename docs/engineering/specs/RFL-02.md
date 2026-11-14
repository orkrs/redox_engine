# RFL-02 — Пробы отражений в уровне: включение, расстановка, захват

- **Статус:** не начат (аудит кода 2026-11-15).
- **Размер:** 4 шага (S, M, M, S) ≈ 6 дней. **Крейты:** `redox_render`, `redox_core`, `redox_editor`. **Зависит от:** RFL-01 (математика, `set_reflection_probes`, `capture_reflection_probe`). **Блокирует:** RFL-03 (запечённые кубы подставляются в те же слоты).
- **Модель:** haiku. Захват (шаг 3) — с проверкой координатора на GPU.
- Пробы начинают отражаться в игре и в viewport без ручного вызова из теста. **Не входит:** HDR-выход граней (ограничение RFL-01, LDR), запекание в KTX2 (RFL-03), обновление каждый кадр, пробы на прозрачных поверхностях.

## Суть
`Renderer::set_reflection_probes_enabled` по умолчанию выключен. `set_reflection_probes` принимает до четырёх коробок и **не** переснимает грани: захват — отдельный `capture_reflection_probe`. Редактор и примеры этот вызов не делают, поэтому локальные отражения есть только в `tests/reflection_probes.rs`.

## Что уже есть
- Компонент пробы и настройки захвата: `redox_render/src/reflection/`.
- После захвата состояние TAA/VSM восстанавливается (`renderer/capture_state.rs`).
- Пока пробы выключены или их нет, кадр совпадает с кадром без проб (контракт `set_reflection_probes_enabled`).

## Шаг 1 — сбор слотов из мира (S)
- Система или функция `probes_from_world(world) -> Vec<ReflectionProbe>`: сущности с компонентом пробы, сортировка как у `set_reflection_probes` (приоритет, не больше 4).
- Грязный флаг компонента: новая проба и сдвиг центра/полуразмера помечают слот к захвату. Сдвиг камеры — нет.
Тесты без GPU: `four_probes_keep_the_highest_priority`; `moving_the_center_marks_the_slot_dirty`; `moving_the_camera_does_not`.

## Шаг 2 — кадр применяет слоты (M)
- На `Stage::Render` до отрисовки: `set_reflection_probes_enabled(true)`, когда в мире есть хотя бы одна проба, иначе `false`. Затем `set_reflection_probes`.
- Игра и редактор регистрируют систему одним плагином `ReflectionProbePlugin` в `redox_core` (по образцу `GiSdfStreamingPlugin`). Без компонента система выключает пробы и выходит.
Тест: `a_world_without_probes_leaves_them_disabled`; `a_world_with_one_probe_enables_them_and_uploads_the_box`.

## Шаг 3 — захват грязных слотов (M)
- Не чаще одного слота за кадр: если слот грязный, `capture_reflection_probe` по объектам кадра и текущему свету, затем снять грязь.
- Загрузка уровня помечает все пробы грязными один раз.
- Редактор: та же система на viewport-мире. Отдельной кнопки «Capture» достаточно как принудительной пометки всех слотов грязными; без кнопки первый кадр после открытия всё равно снимает.
Тест GPU: `a_red_wall_shows_up_in_the_probe_after_load` (тот же критерий, что в RFL-01: направление на красную стену, красный канал > 0.5). `the_frame_after_capture_matches_the_frame_without_capture` остаётся зелёным.

## Шаг 4 — документация (S)
`PLAN_LIGHTING.md`: пробы выключены, пока в мире нет компонента. `CHANGELOG.md`. В `render_host.rs` не писать, что захвата нет, после того как система стоит на viewport.

## Границы
- **Читать:** RFL-01, `reflection/{probe,component,capture}.rs`, `renderer/settings.rs`, `renderer/capture_state.rs`.
- **Создать:** `redox_core` плагин проб (файл рядом с `gi_streaming.rs`).
- **Не трогать:** формулу параллакса, лимит 4, пост-цепочку захвата (HDR — отдельное ограничение).

## Проверка
`cargo fmt`, `cargo clippy -p redox_render -p redox_core -p redox_editor --all-targets -- -D warnings`, `cargo test -p redox_render --test reflection_probes`, тесты плагина без GPU.

## Для исполнителя
`git merge --ff-only main`. Если компонент пробы не виден из `redox_core` из-за слоёв крейтов — плагин остаётся в крейте, который уже зависит от обоих, и это фиксируется в отчёте. Не поднимать лимит четырёх проб.
