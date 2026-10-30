# PST-07 — TSR: проверка и доводка временного апскейла (TAA при `render_scale < 1`)

- **Статус:** реализовано (шаги 1–4) в ветке worktree; код и тесты не компилировались и не запускались (запрет волны), проверка координатором: fmt, clippy, `cargo test --test tsr`, PSNR/призрак по тестам.

- **Размер:** 4 шага (S, M, M, S) ≈ 7 дней. **Крейт:** `redox_render`. **Зависит от:** PST-04 (метрики TAA, готово). **Блокирует:** —.
- **Модель:** haiku по шагам 1, 4; шаги 2–3 (шейдер) — haiku с проверкой координатора.
- Объём — «минимум» C1 из `PLAN_EXECUTION.md`: апскейл до ~67 % по каждой оси с валидацией истории. **Не входит:** FSR/XeSS/DLSS (опциональные плагины, `PLAN_LIGHTING.md` §6.2), динамическое разрешение по времени кадра.

## Суть и аудит
- Есть: `Renderer::set_render_scale(scale)` (0,5…2,0), `uses_tsr() = taa_enabled && render_scale < 0.999` (`renderer/accessors.rs:52–83`); `taa.wgsl` принимает `jitter_uv`, `input_size` (читает вход меньшего размера, сэмплируя с поправкой на джиттер); YCoCg-клиппинг, реактивная маска, сверка глубины прошлого кадра (`PLAN_EXECUTION`).
- Нет: **ни одного теста апскейла** (PST-04 мерил TAA на 1,0), нет резкости после апскейла, джиттер — последовательность и её связь с `render_scale` не проверены, нет LOD-bias для текстур, нет отчёта качества. Поэтому TSR считать «готовым» нельзя; цель этой спеки — доказать числами или доработать.
- Backlog (`BACKLOG.md` §4.2) отмечал, что апскейл 67 % для 1080p на RTX 4060 Ti не нужен: цель — иметь **работающий и проверенный** режим для слабых машин и 4K, а не ускорять 1080p.

## Шаг 1 — эталон качества и чистая математика (S)
- `post/tsr.rs` (чистая математика, зеркало WGSL): `jitter_sequence(index: u32, render_scale: f32) -> [f32; 2]` (Halton(2,3), число фаз `ceil(8 / scale²)` ≥ 8, диапазон ±0,5 пикселя входного размера), `texture_lod_bias(render_scale) -> f32 = log2(scale) - 1.0` (минимум −2.0), `sharpen_amount(render_scale) -> f32` (0 при 1,0; растёт до 0,35 при 0,5).
- Тест-эталон `tests/tsr.rs` (GPU): сцена — шахматная плоскость + тонкие линии (1 px), статическая камера. Метрики (функции в `tests/common` или в самом тесте): `psnr(a, b)` и `edge_sharpness(img)` (средний градиент по Собелю).
```rust
#[test] fn halton_jitter_has_no_repeats_within_a_cycle_and_stays_in_range() { /* юнит */ }
#[test] fn the_jitter_phase_count_grows_as_the_scale_shrinks() { /* scale 1.0 -> 8, 0.67 -> 18, 0.5 -> 32 */ }
#[test] fn the_lod_bias_is_negative_below_native_and_clamped() { /* ... */ }
#[test] fn upscaling_a_static_scene_converges_towards_the_native_taa_frame() { /* 64 кадров при scale 0.67: PSNR к кадру scale=1.0 TAA ≥ 28 дБ */ }
#[test] fn the_upscaled_frame_is_not_blurrier_than_70_percent_of_native_sharpness() { /* edge_sharpness(0.67) ≥ 0.7 * edge_sharpness(1.0) */ }
```
Если тесты сходимости/резкости красные на **текущем коде** — это находка: записать числа, остановиться, шаги 2–3 по решению координатора.

## Шаг 2 — джиттер и LOD bias по масштабу (M)
- Источник джиттера (**ПРОВЕРИТЬ** `pass/taa.rs`, `renderer/aa.rs`): использовать `jitter_sequence` с числом фаз, зависящим от масштаба; джиттер в долях входного разрешения.
- `lod_bias` в сэмплерах материалов (**ПРОВЕРИТЬ** создание сэмплера в `resource/texture.rs`/`renderer/init.rs`, поле `lod_bias` у `wgpu::SamplerDescriptor` отсутствует → bias применять в шейдере `textureSampleBias` или через uniform-смещение в `pbr.wgsl`; выбрать вариант с меньшей правкой, записать выбор).
- При `render_scale = 1.0` кадр побитно прежний (golden).
Тесты (GPU): `jitter_matches_the_formula_in_the_uniform` (прочитать uniform через staging), `lod_bias_is_zero_at_native_scale`, `a_distant_texture_stays_sharper_with_the_bias_at_067` (дисперсия высоких частот в области шахматки выше с bias).

## Шаг 3 — резкость и валидация истории при апскейле (M)
- `taa.wgsl`: после накопления — контрастно-адаптивная резкость (CAS-подобный фильтр 3×3, коэффициент `sharpen_amount`), отключается при scale ≥ 0,999 и при быстром движении (по скорости).
- Репроекция истории при апскейле: окно клиппинга YCoCg расширить на `1/scale` пикселей входа; отклонение истории по глубине — порог масштабируется на `1/scale` (иначе ложные отклонения на краях). **ПРОВЕРИТЬ** текущие пороги в `taa.wgsl` (строки ~180–270) и записать старые/новые значения в отчёт.
Тесты: `the_sharpen_pass_raises_edge_sharpness_but_not_above_native_plus_20_percent` (избегать звона), `no_ringing_on_a_single_pixel_line` (значение рядом с линией не превышает максимум соседей > 5%), метрики PST-04 `moving_object_leaves_no_ghost_at_067` (призрак в допуске PST-04, умноженном на 1,5), `scale_one_is_bitwise_unchanged`.

## Шаг 4 — замер и документация (S)
`tests/tsr.rs::bench` (`#[ignore]`): 1080p и 4K-цель, scale 1,0 / 0,77 / 0,67 / 0,5; GPU-время кадра; печатать мс и PSNR к нативу. Абзац в `PLAN_LIGHTING.md` §6.1 («что есть»), `PLAN_EXECUTION.md` C1, `CHANGELOG.md`, `docs/BENCHMARKS.md`. Качество не «улучшать до идеала»: цель — PSNR ≥ 28 дБ при 0,67 на статике и отсутствие призрака.

## Границы
- **Читать:** `renderer/{accessors,aa,post}.rs`, `pass/taa.rs`, `shaders/taa.wgsl`, `tests/{post_fx,wave7_gpu}.rs` (метрики PST-04).
- **Создать:** `post/tsr.rs`, `tests/tsr.rs`.
- **Менять:** `shaders/taa.wgsl`, `shaders/pbr.wgsl` (bias, если шейдерный вариант), `pass/taa.rs`, `renderer/aa.rs`.
- **Не трогать:** SSR, туман, DoF, VG.

## Проверка
`cargo fmt`, `cargo clippy -p redox_render --all-targets -- -D warnings`, `cargo test -p redox_render --test tsr --test post_fx --test golden_frames`, `cargo check --workspace --all-targets`, `python scripts/check_file_sizes.py`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Шаг 1 — только тесты и чистая математика, красные тесты в отчёт и остановка (решает координатор). Допуски не ослаблять.
