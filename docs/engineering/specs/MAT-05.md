# MAT-05 — Материалы, E5: подповерхностное рассеяние (SSS) — wrap-освещение и экранное разделяемое SSS

- **Статус:** выполнены шаги 1–2; шаг 3 (второй вывод и размытие) не сделан.

- **Размер:** 4 шага (S, M, M, S) ≈ 7 дней. **Крейт:** `redox_render`. **Зависит от:** MAT-01 (`SurfaceData`). Граф не требуется (параметры — поля `Material`). **Блокирует:** MAT-07.
- **Модель:** haiku по шагам 1, 2, 4; шаг 3 (второй цветовой вывод и MSAA) — haiku **с проверкой координатора**.
- Объём — «минимум» плана E5: wrap + экранное разделяемое SSS. **Не входит:** пропускание на просвет (transmission: уши, листья на солнце), карты толщины.

## Суть
SSS — свет входит в материал, рассеивается и выходит рядом (кожа, воск, мрамор): мягкий терминатор, подсветка «изнутри». Пропускание «насквозь» — другой эффект, в этой спеке не делается (причина — в плане: нужна толщина).
Реализуем по возрастанию стоимости.

## Что уже есть (проверено чтением)
- Основной проход пишет один HDR-цвет (`begin_pass`, `renderer/draw.rs:8–36`: один `color_attachment` + MSAA-resolve, `hdr_msaa_view`/`hdr_view`).
- Глубина (`depth_view`), нормаль+линейная глубина (`normal_view`, префасс), GTAO — пост-проходы на готовых текстурах (шаблоны разделяемого размытия: `shaders/gtao_blur.wgsl`).
- `SurfaceData` после MAT-01; материал без SSS должен давать побитно прежний кадр.

## Шаг 1 — параметры и wrap-освещение (S)
- `SubsurfaceProfile { color: [f32;3] /* цвет рассеяния */, radius_mm: f32 /* 0..20 */, wrap: f32 /* 0..1 */ }`; `Material::subsurface(profile)`, поле в `Material`/`MaterialUniform` (в конец; обновить тест размера).
- Wrap: диффузный член `n·l` заменить на `max((n·l + w) / (1 + w), 0)` (с нормировкой) для материалов с `wrap > 0`; цвет терминатора — `mix(vec3(1), profile.color, wrap)` на границе (`saturate(−n·l)`-зона). Для `wrap == 0` — прежний код (ветка, побитно).
- Эталон формулы в Rust: `pub fn wrap_diffuse(n_dot_l: f32, wrap: f32) -> f32` (`post/sss.rs`) + WGSL зеркало.
Тесты (юнит):
```rust
#[test] fn zero_wrap_equals_the_lambert_term() { /* wrap_diffuse(x,0) == max(x,0) */ }
#[test] fn wrap_lights_the_shadow_side_a_little() { /* n_dot_l = -0.2, wrap 0.5 -> > 0 */ }
#[test] fn wrap_is_normalised_so_full_light_stays_one() { /* n_dot_l = 1 -> 1.0 */ }
#[test] fn wrap_is_monotone_in_n_dot_l() { /* sweep */ }
```
GPU: `a_wrapped_sphere_has_a_softer_terminator` (сфера под боковым светом: яркость в 10° за терминатором с wrap 0.5 заметно выше, чем с 0, > +0.05; на освещённой стороне разница ≤ 0.02); `a_material_without_sss_renders_as_before` (golden).

## Шаг 2 — ядро размытия и математика профиля (M, чистая логика)
`post/sss.rs`: ядро Jimenez (разделяемое, по каналам):
```rust
/// One sample of the separable kernel: offset in kernel units (-1..1) and an RGB weight.
pub struct KernelSample { pub offset: f32, pub weight: [f32; 3] }
/// Builds an odd-sized (taps = 7/9/11) kernel for the scattering colour `profile` (per-channel falloff). Weights sum to 1 per channel.
pub fn sss_kernel(profile: [f32; 3], taps: usize) -> Vec<KernelSample>;
/// Blur width in pixels at view distance `dist_m` for `radius_mm`, on a screen of `screen_h` pixels with the vertical field of view `fov_y`.
pub fn blur_radius_pixels(radius_mm: f32, dist_m: f32, fov_y: f32, screen_h: f32) -> f32;
```
Профиль гауссов: ширина канала ∝ `profile[c]` (красный шире синего); `offset` — нелинейное распределение (плотнее около центра), нормировка весов по каналам.
Тесты:
```rust
#[test] fn kernel_weights_sum_to_one_per_channel() { /* taps 7, 9, 11 */ }
#[test] fn the_kernel_is_symmetric() { /* sample[i] == sample[n-1-i] по weight и -offset */ }
#[test] fn the_centre_sample_has_the_largest_weight() { /* ... */ }
#[test] fn red_scatters_wider_than_blue() { /* профиль (0.9,0.4,0.2): сумма |offset|*weight по R > по B */ }
#[test] fn the_radius_in_pixels_falls_with_distance_and_grows_with_radius() { /* ... */ }
#[test] fn a_zero_radius_gives_a_zero_blur() { /* ... */ }
#[test] fn degenerate_inputs_do_not_produce_nan() { /* dist 0, fov 0 */ }
```

## Шаг 3 — второй цветовой вывод и проходы размытия (M)
1. Основной проход: **второй** цветовой вывод `Rgba16Float` (`sss_diffuse`: rgb — диффузная составляющая освещения SSS-пикселя; a — маска 1). Для не-SSS материалов пишется `(0,0,0,0)`, а их диффузное идёт в основной цвет как раньше; для SSS-пикселей диффузное **не** добавляется в основной цвет (только спекуляр и окружение). MSAA: второй вывод тоже MSAA с resolve (`hdr_sss_msaa_view`/`hdr_sss_view`); пересоздание в `resize.rs` рядом с `hdr_view`. Включается только если в кадре есть SSS-материалы (флаг на `Renderer`, иначе второй вывод не создаётся/не пишется и кадр побитно прежний).
   **ПРОВЕРИТЬ** влияние на конвейеры: PBR, прозрачный (`pass/transparent.rs`: цели `targets: &[Some(..)]` — прозрачный проход не должен писать в SSS-буфер → второй target с `write_mask: empty`), VG-resolve (пишет в тот же HDR: SSS для VG — MAT-07).
2. Два прохода `shaders/sss_blur.wgsl` (горизонт, вертикаль): читают `sss_diffuse`, глубину (линейную из `normal_view.a * 1000`), профиль (на пиксель: цвет и радиус — из `sss_diffuse.a`/второго канала; **упрощение v1**: один профиль на кадр, берётся из первого SSS-материала сцены, документировать), вес выборки обнуляется при разнице глубин > `depth_tolerance` (не размывать через силуэт). Результат — в `sss_blur` (ping-pong), затем сложение: `hdr_color += sss_blurred.rgb * albedo?` — albedo уже учтён при записи диффузного (рассеивать лучше освещённость, но v1 размывает цвет диффузного, что допустимо; ограничение записать).
3. Интеграция в `PostFx`-цепочку или отдельный вызов в `record_post_chain` перед TAA (SSS работает **до** TAA, на HDR кадра с джиттером — иначе размытые границы мерцают): **ПРОВЕРИТЬ** место вставки (`renderer/frame/post.rs`).
4. `Renderer::set_sss(enabled: bool)` (по умолчанию включается автоматически, если есть SSS-материалы; явное `false` — выключает и возвращает обычное диффузное).
Тесты (GPU, `tests/sss.rs`): `a_frame_without_sss_materials_pays_nothing` (флаг: число проходов SSS = 0; кадр побитно прежний); `a_skin_sphere_has_a_reddish_soft_terminator` (красный канал на границе света/тени выше синего на ≥ 0.03 относительно сферы без SSS); `the_silhouette_is_not_blurred` (резкий край сферы на тёмном фоне: ширина перехода яркости не больше, чем без SSS + 1 пиксель); `sss_survives_a_resize`; `msaa_and_non_msaa_both_work` (если конфигурации есть).

## Шаг 4 — документация и замер (S)
Абзац в `PLAN_MATERIALS.md` §3, `PLAN_EXECUTION.md` E5, `CHANGELOG.md`. Замер (в отчёт): стоимость двух проходов на 1080p при сфере на пол-экрана (GPU-таймер, если доступен; иначе CPU-время кадра); цель ≤ 0,5 мс.

## Границы
- **Читать:** `renderer/draw.rs:1–40`, `renderer/frame/post.rs`, `renderer/{init,resize}.rs`, `pass/{pbr,transparent}.rs`, `shaders/gtao_blur.wgsl`, `pbr.wgsl` (диффузный член), `PLAN_MATERIALS.md` §3.
- **Создать:** `post/sss.rs`, `shaders/sss_blur.wgsl`, `tests/sss.rs`.
- **Менять:** `material.rs`, `pbr.wgsl`/`material_surface.wgsl`, `pass/{pbr,transparent}.rs`, `renderer/*`, `shader/mod.rs`.
- **Не трогать:** тени, VG (MAT-07), TAA.

## Проверка
`cargo fmt`, `cargo clippy -p redox_render --all-targets -- -D warnings`, `cargo test -p redox_render --lib sss`, `cargo test -p redox_render --test sss --test golden_frames --test resize_bloom --test post_fx`, `python scripts/check_file_sizes.py`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Шаги 1–2 не меняют кадры существующих материалов (побитно). Если второй цветовой вывод ломает какую-либо конфигурацию конвейеров — остановись и опиши список затронутых конвейеров.
