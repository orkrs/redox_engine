# PST-03 — Motion blur: расширение скорости за контур объекта

- **Статус:** реализовано (шаги 1–3) в ветке worktree, не скомпилировано и не запускалось: cargo, clippy, тесты и замер GPU — за координатором. Флаг `spread_beyond_edges` по умолчанию выключен. Не сделано: тест «тайловый проход находит скорость куба» (нужен readback тайлов), тесты протяжённости и статического объекта за быстрым.

- **Размер:** 3 шага (S, M, S) ≈ 3–4 дня. **Крейт:** `redox_render`. **Зависит от:** PST-01 (готов). **Блокирует:** —.
- **Модель:** haiku по шагам 1, 3; шаг 2 (шейдеры) — haiku с проверкой координатора.
- Закрывает упрощение PST-01: «блюр применяется к пикселям движущегося объекта; контур наружу не растёт» — внешний край быстро летящего предмета остаётся резким. **Не входит:** размытие фона сквозь объект (вклад заднего плана), тайловая оптимизация сверх описанной.

## Суть
Классическое решение: (1) разбить кадр на тайлы 20×20 и взять в каждом **максимальную скорость** (`tile_max`), (2) расширить на соседние тайлы (`neighbor_max` = максимум по 3×3), (3) в основном проходе пиксель, не движущийся сам, но рядом с быстрым тайлом, получает размытие вдоль скорости соседа, если сосед «дотягивается» до него (`|v| * T ≥ расстояние`) и находится **ближе** пикселя к камере (сосед заслоняет).
В `motion_blur.wgsl` сейчас: сэмплы вдоль **собственной** скорости пикселя (`motion_blur_offsets` в `post_fx.rs:67`).

## Шаг 1 — чистая математика и эталон (S)
`post/motion_blur_tiles.rs` (зеркало WGSL):
```rust
pub const TILE_SIZE: u32 = 20;
/// Largest-magnitude velocity in a tile (pixels); ties keep the first.
pub fn tile_max(velocities: &[[f32; 2]]) -> [f32; 2];
/// Max over the 3×3 tiles around (tx,ty).
pub fn neighbor_max(tile_maxima: &[[f32; 2]], tiles_x: u32, tiles_y: u32, tx: u32, ty: u32) -> [f32; 2];
/// Weight with which a neighbour's blur reaches pixel `p`: 0 when the neighbour velocity (scaled by exposure `shutter`) cannot cover the distance, smooth falloff near the end; 0 when the neighbour is farther from the camera than `p` (depth compare with `soft_z`).
pub fn spread_weight(neighbor_velocity: [f32; 2], dist_pixels: f32, depth_p: f32, depth_neighbor: f32, shutter: f32, soft_z: f32) -> f32;
```
Тесты:
```rust
#[test] fn the_tile_max_is_the_longest_velocity() { /* ... */ }
#[test] fn the_neighbour_max_covers_all_eight_neighbours_and_the_centre() { /* ... */ }
#[test] fn the_edges_of_the_image_have_fewer_neighbours_without_a_panic() { /* ... */ }
#[test] fn a_neighbour_too_slow_to_reach_the_pixel_has_zero_weight() { /* ... */ }
#[test] fn a_neighbour_behind_the_pixel_does_not_blur_it() { /* depth_neighbor > depth_p + soft_z */ }
#[test] fn the_weight_falls_smoothly_to_zero_at_the_reach_limit() { /* монотонно, без скачка > 0.15 за пиксель */ }
```

## Шаг 2 — проходы и шейдер (M)
- `shaders/motion_blur_tiles.wgsl`: два compute-прохода — `tile_max` (по 20×20 берёт максимум из `velocity` текстуры, `Rg16Float`, размер `ceil(w/20) × ceil(h/20)`) и `neighbor_max` (3×3). Текстуры пересоздаются в `resize.rs`.
- `motion_blur.wgsl`: для пикселя `p`: свои сэмплы как сейчас (если собственная скорость > порога); иначе (или дополнительно, если скорость соседа больше) — берёт `neighbor_max` своего тайла, направление размытия = направление соседа, число сэмплов — как для скорости соседа; вес вклада каждого сэмпла — `spread_weight` с проверкой глубины сэмпла; итог смешивается с цветом пикселя по весу.
- Включается только при `MotionBlurSettings::spread_beyond_edges = true` (по умолчанию **false**, пока не принято владельцем визуально); `false` — кадр побитно прежний.
Тесты (GPU, `tests/post_fx.rs` дополнить или `tests/motion_blur_spread.rs`):
```rust
#[test] fn the_tile_pass_finds_the_fast_objects_velocity() { /* куб 10 px/кадр: tile_max в его тайле == скорость ±1e-3 */ }
#[test] fn the_edge_outside_a_fast_object_is_blurred_with_spreading_on() { /* ширина подъёма яркости на внешней стороне контура: больше, чем без spreading, ≥ 3 px */ }
#[test] fn a_static_object_behind_a_fast_one_is_not_smeared_over_the_front_object() { /* ... */ }
#[test] fn without_the_flag_the_frame_is_bitwise_unchanged() { /* golden/существующие тесты post_fx */ }
#[test] fn the_blur_extent_matches_the_velocity() { /* протяжённость размытия вне контура ≈ |v|*shutter ± 2 px */ }
```

## Шаг 3 — документация (S)
`PLAN_LIGHTING.md` §5 (убрать упрощение), `PLAN_EXECUTION.md` (C6), `CHANGELOG.md`; замер GPU-стоимости двух новых проходов (`#[ignore]` тест) в `docs/BENCHMARKS.md`.

## Границы
- **Читать:** `post/post_fx.rs`, `shaders/motion_blur.wgsl`, `renderer/{post,resize}.rs`, `tests/post_fx.rs`, PST-01.
- **Создать:** `post/motion_blur_tiles.rs`, `shaders/motion_blur_tiles.wgsl`, `tests/motion_blur_spread.rs`.
- **Менять:** `post/post_fx.rs`, `shaders/motion_blur.wgsl`, `renderer/{resize,post}.rs`, `shader/mod.rs`.
- **Не трогать:** DoF, TAA.

## Проверка
`cargo fmt`, `cargo clippy -p redox_render --all-targets -- -D warnings`, `cargo test -p redox_render --test post_fx --test motion_blur_spread --test golden_frames`, `cargo check --workspace --all-targets`, `python scripts/check_file_sizes.py`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Флаг по умолчанию `false`; кадры без флага — побитно прежние.
