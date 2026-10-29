# PST-04 — Проверка и доводка TAA на движущихся объектах (C1, «TSR» без апскейла)

- **Размер:** S (измерение) + S (правки, только если метрики не пройдены). **Крейт:** `redox_render`. **Зависит от:** PST-01 (готово). **Блокирует:** ничего.
- **Модель:** haiku. Файлы `shaders/taa.wgsl`, `pass/taa.rs` — одному исполнителю.

## Суть
В плане C1 — «TSR». Аудит показал, что `taa.wgsl` (274 строки) уже содержит большую часть того, что называют TSR-валидацией: клиппинг истории в YCoCg (`clip_aabb`, строка 128, применяется в 242),
Catmull-Rom выборку истории (65), расширение скорости по ближайшей глубине на силуэтах (`dilated_velocity`, 141), отсев по глубине прошлого кадра (`depth_ok`, 250), маску реактивности (`t_reactive`, 265–270),
ограничение веса истории (255) и сброс. **Не хватает не алгоритма, а измерения**: нет теста, который докажет, что за движущимся объектом нет «призрака» и что статичная сцена не мерцает.
Апскейл с внутренним разрешением 67 % (настоящий TSR) в спеку **не входит**: на 1080p и RTX 4060 Ti он не нужен (решение владельца, `BACKLOG.md` §4). Вернуться к нему, если цель станет 4K.

## Что уже есть
- Тест `tests/object_velocity.rs` (сцена с движущимся объектом, чтение текстуры скорости `renderer.taa_velocity_tex`), `tests/golden_frames.rs` (цикл джиттера: «рисует кадры целого цикла»), `tests/post_fx.rs` (рендер с TAA, `common::adapter()`, `common::renderer`).
- `Renderer::enable_taa(bool)`.

## Метрики (чистые функции, юнит-тесты без GPU) — `tests/common/taa_metrics.rs`
```rust
/// Pixels of `row` (RGBA8, `width` wide) in `x_range` whose luminance is above `background + threshold` (0..1 luminance).
pub fn bright_pixels(frame: &[u8], width: usize, row: usize, x_range: std::ops::Range<usize>, background: f32, threshold: f32) -> usize;
/// Mean absolute per-channel difference (0..1) of two equally sized RGBA8 frames, ignoring alpha.
pub fn mean_abs_diff(a: &[u8], b: &[u8]) -> f32;
/// Largest per-channel difference (0..255) between two frames.
pub fn max_channel_diff(a: &[u8], b: &[u8]) -> u8;
```
Юнит-тесты метрик (в `tests/common/taa_metrics.rs` под `#[cfg(test)]` или в отдельном тесте): одинаковые кадры → 0; один изменённый пиксель → `max_channel_diff == разница`; `bright_pixels` считает ровно число засвеченных пикселей строки.

## Тесты на GPU (`tests/taa_motion.rs`, пропуск без адаптера; сцена — по `tests/object_velocity.rs`)
Фон тёмный однотонный (небо отключить/залить; **ПРОВЕРИТЬ:** как `object_velocity.rs` получает однотонный фон), камера неподвижна, яркий куб (`Material::solid`, эмиссия или белый при сильном свете), TAA включён.
```rust
#[test] fn a_moving_cube_leaves_no_ghost_trail() {
    // куб идёт вдоль X по 0.15 единицы/кадр 40 кадров; после последнего кадра в строке через центр куба посчитать bright_pixels
    // в области ЗА кубом (по ходу движения назад) шириной 60 пикселей, начиная с 3 пикселей от заднего края куба.
    // assert!(trail <= 4, "ghost trail {trail} px")  // допуск 4 пикселя на сглаженный край
}
#[test] fn a_static_scene_is_stable_over_a_jitter_cycle() {
    // 16 кадров статичной сцены (после 8 прогревочных); max mean_abs_diff между соседними кадрами < 0.004 (≈1/255)
}
#[test] fn a_still_after_motion_converges() {
    // куб движется 10 кадров, потом стоит 24 кадра: разница последних двух кадров < 0.004
}
#[test] fn a_new_object_entering_the_view_is_not_smeared() {
    // куб появляется в кадре N (раньше его нет в списке); на кадре N+1 bright_pixels в области куба >= 80% от кадра N+20 (нет «проявления» из тьмы)
}
```
Пороги — стартовые; исполнитель сначала **печатает** значения (`eprintln!`) на текущем коде и отчитывается. Если стартовые пороги пройдены — правки шейдера не нужны, спека закрыта. Если нет — шаг ниже.

## Шаг 2 — правки (только если тест красный)
Менять по одному параметру, после каждого — повторный запуск всех четырёх тестов и `golden_frames`:
1. Призрак: уменьшить предельный вес истории (`taa.wgsl:255`, комментарий «0.97 was too soft») и/или усилить `lum_diff`-блокировку (235–243) на движении (`vel_len` из 227).
2. Мерцание: увеличить вес истории на неподвижных пикселях (`vel_conf`, 214) — только при `vel_len < 0.1` пикселя.
3. Новый объект: при `depth_ok < 0.5` (отсев по глубине) брать текущий кадр без блока истории — уже так (261), проверить, что это срабатывает для впервые видимых пикселей.
`golden_frames` — эталоны не обновлять; если правка меняет картинку, остановиться и отчитаться (решение о смене эталона — владельца).

## Границы
- **Читать:** `shaders/taa.wgsl`, `pass/taa.rs`, `tests/object_velocity.rs`, `tests/golden_frames.rs`, `tests/common/mod.rs`, `tests/post_fx.rs`.
- **Создать:** `tests/common/taa_metrics.rs` (+ `mod taa_metrics;` в `tests/common/mod.rs`), `tests/taa_motion.rs`.
- **Менять:** шаг 2 — `shaders/taa.wgsl` (и `pass/taa.rs`, если нужен новый параметр).
- **Не трогать:** остальные пост-эффекты, голдены.

## Проверка
`cargo fmt`, `cargo clippy -p redox_render --all-targets -- -D warnings`, `cargo test -p redox_render --test taa_motion --test golden_frames --test object_velocity --test post_fx`.
Живая проверка (владелец, QA-01): в Foxfire быстро повернуть камеру и пробежать героем мимо тёмного фона — нет шлейфа, нет дрожи краёв.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Если метрики пройдены с первого раза — так и сообщи (это нормальный результат), в `BACKLOG.md` ничего не правь.
