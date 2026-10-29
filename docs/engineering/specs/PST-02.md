# PST-02 — Глубина резкости (DoF)

- **Статус:** выполнено 2026-10-29, принято в `main`. Стандартный порядок Z подтверждён (`linear_distance`).

- **Размер:** M. **Крейт:** `redox_render`. **Зависит от:** PST-01 (каркас `PostFx` с двумя текстурами и `run_chain`). **Блокирует:** ничего.
- **Модель:** sonnet.

## Суть
Нужен простой и дешёвый DoF: размытие по кругу пропорционально «кругу нерезкости» (CoC), посчитанному из глубины. Без разделения на слои переднего/заднего плана,
без боке-форм (они в плане режутся первыми). Эффект включается как второй шаг цепочки `PostFx` после motion blur.

## Границы
- **Читать:** `post/post_fx.rs` и шейдер `motion_blur.wgsl` из PST-01 (образец), `renderer/frame/post.rs` (место вызова `run_chain`), `camera.rs`/`FrameView` (где лежат `near`/`far`;
  `Renderer::camera_near`, `camera_far` задаются в `render_frame_view`), `renderer/mod.rs` (`depth_texture`, формат `Depth32Float`, `depth_view`).
- **Создать:** `shaders/dof.wgsl`, `tests/dof.rs`. **Менять:** `post/post_fx.rs`, `renderer/settings.rs`, `renderer/frame/post.rs` (только вызов), `renderer/init.rs`, `renderer/resize.rs` (если `DofPass` держит bind group).
- **Не трогать:** TAA, bloom, tone mapping, golden-эталоны (при выключенном DoF — побитно те же кадры).

## Решение
### Чистая математика (юнит-тесты без GPU)
```rust
/// Parameters of the depth of field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DofSettings {
    /// Distance from the camera that is perfectly sharp, in world units.
    pub focus_distance: f32,
    /// Lens strength: circle of confusion in pixels per unit of relative defocus (see `coc_pixels`).
    pub aperture: f32,
    /// Largest blur radius in pixels (the cost is bounded by it).
    pub max_radius_pixels: f32,
}
impl Default for DofSettings { /* focus_distance 10.0, aperture 6.0, max_radius_pixels 8.0 */ }

/// Radius of the circle of confusion in pixels for a pixel at linear distance `distance` (>0).
/// `coc = clamp(aperture * |distance - focus| / distance, 0, max_radius)`; a non-finite or non-positive distance gives 0.
pub fn coc_pixels(distance: f32, s: &DofSettings) -> f32;

/// Linear view distance from a reversed-or-standard depth value; **проверить** в коде, какой Z (стандартный или reversed) использует рендерер
/// (`camera.rs`, `projection`), и реализовать под него. Для стандартной перспективы: `d = near*far / (far - z*(far-near))`.
pub fn linear_distance(depth: f32, near: f32, far: f32) -> f32;

/// Disc sample offsets (unit disc) for `n` taps: sunflower/golden-angle spiral, `n >= 1`, first tap at the centre.
pub fn disc_taps(n: u32) -> Vec<[f32; 2]>;
```
### Шейдер `dof.wgsl`
Биндинги: `0` цвет (источник цепочки), `1` глубина (`texture_depth_2d`, `textureLoad`), `2` sampler, `3` uniform `{ screen, near, far, focus, aperture, max_radius, taps, _pad }`.
Для пикселя: `coc = coc_pixels(...)`; если `coc < 0.5` → вернуть цвет; иначе 16 тапов `disc_taps(16)` в радиусе `coc` пикселей; вес тапа = `1`, **но** тап чётче центра
(его собственный CoC меньше расстояния до центра) вес 0 — это убирает «просачивание» резкого переднего плана на размытый фон. Среднее по весам.
### Управление
`Renderer::set_depth_of_field(Option<DofSettings>)` и геттер; по умолчанию `None`. В `PostFx::run_chain` порядок: motion blur (если включён) → DoF (если включён); текстуры ping-pong `A→B`.
Если включён только DoF, источником служит `taa_history[write]`, итог в `textures[0]`; индекс результата возвращается как в PST-01.

## Тесты
Юнит:
```rust
#[test] fn the_focus_plane_is_sharp() { assert_eq!(coc_pixels(10.0, &DofSettings::default()), 0.0); }
#[test] fn the_blur_grows_away_from_focus_and_is_capped() {
    let s = DofSettings { focus_distance: 10.0, aperture: 6.0, max_radius_pixels: 8.0 };
    assert!(coc_pixels(5.0, &s) > 0.0);
    assert!(coc_pixels(2.0, &s) >= coc_pixels(5.0, &s));
    assert_eq!(coc_pixels(0.5, &s), 8.0);
    assert!(coc_pixels(1.0e6, &s) <= 8.0, "far background is capped by max_radius_pixels");
}
```
```rust
#[test] fn bad_distances_give_zero() {
    let s = DofSettings::default();
    for d in [0.0, -1.0, f32::NAN, f32::INFINITY] { assert_eq!(coc_pixels(d, &s), 0.0, "{d}"); }
}
#[test] fn disc_taps_stay_inside_the_unit_disc_and_start_at_the_centre() {
    let t = disc_taps(16);
    assert_eq!(t.len(), 16);
    assert_eq!(t[0], [0.0, 0.0]);
    assert!(t.iter().all(|p| (p[0] * p[0] + p[1] * p[1]).sqrt() <= 1.0 + 1e-6));
}
#[test] fn linear_distance_matches_near_and_far() {
    // depth 0 -> near, depth 1 -> far for the convention used by the renderer (adjust to reversed Z if the camera uses it)
    assert!((linear_distance(0.0, 0.1, 100.0) - 0.1).abs() < 1e-4);
    assert!((linear_distance(1.0, 0.1, 100.0) - 100.0).abs() < 1e-1);
}
```
GPU (`tests/dof.rs`, пропуск без адаптера):
```rust
#[test] fn with_dof_off_the_frame_is_unchanged() { /* two renderers, identical frames, pixel-identical */ }
#[test] fn a_far_object_loses_its_edge_sharpness_while_the_focused_one_keeps_it() {
    // two thin bright vertical bars (cuboids) at distance 10 and distance 40, focus 10.
    // Edge sharpness = max horizontal gradient of luminance across the bar edge.
    // With DoF ON: gradient(near bar) >= 0.9 * gradient(OFF); gradient(far bar) <= 0.6 * gradient(OFF).
}
#[test] fn dof_and_motion_blur_can_run_together() { /* both Some(..): no validation error, output not black, differs from each alone */ }
```

## Проверка
`cargo test -p redox_render --lib post_fx`, `--test dof --test motion_blur --test golden_frames`, clippy, fmt. В отчёт: стоимость кадра (цель ≤ 0,5 мс на 1080p).

## Для исполнителя
`git merge --ff-only main`; убедись, что PST-01 в `main`. Порядок Z (стандартный/reversed) проверь в коде **до** написания `linear_distance`; тест `linear_distance_matches_near_and_far` подгони под реальную конвенцию, не наоборот.
