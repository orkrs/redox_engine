# RFL-01 — C4: пробы отражений (захват в движке, коробочная параллакс-коррекция, смешивание)

- **Статус:** шаги 1–4 выполнены 2026-10-29, 8 тестов проходят; шаг 5 (KTX2) не делался. Известно: захват в LDR (переход на `Rgba16Float` не локален: пост-цепочка пишет в формат поверхности, нужен линейный HDR-выход граней); параллакс проверен слабо (кадры различаются); теста плавного смешивания двух проб (монотонность через зону смешивания) нет, есть только тест равных весов `two_overlapping_probes_with_equal_weights_average_their_reflections`.
- **Захват побочно-нейтрален (исправлено, не проверено запуском):** причина — грани идут через обычный кадровый путь, который менял TAA-счётчик и ping-pong, историю TAA и предыдущую глубину, историю VSM screen-shadow, клипмап теней (центрируется на точке пробы), page feedback и историю движения объектов; восстанавливались только камера и свет. Гипотеза по уверенности: высокая для TAA-счётчика/истории (сдвиг фазы джиттера и ghosting ~16 кадров при alpha≈0.1), средняя для клипмапа и feedback. Исправление — `renderer/capture_state.rs` (сохранить/восстановить скаляры и текстуры, отключить клипмап/feedback/motion на время граней). Тест `the_frame_after_a_capture_equals_the_frame_without_one` (`tests/reflection_probes.rs`) написан, не запускался; возможная недетерминированность асинхронного feedback в тесте — `// ASSUMPTION` в отчёте.

- **Размер:** 5 шагов (S, S, M, M, S) ≈ 6–7 дней. **Крейт:** `redox_render` (шаг 5 — ещё `redox_cook`). **Зависит от:** — (PST-05 SSR уже в `main`). **Блокирует:** GI-04.
- **Модель:** haiku по шагам 1, 2, 5; шаги 3–4 (захват, шейдер) — haiku с проверкой координатора после каждого шага.
- Объём — «минимум» плана C4 (`PLAN_EXECUTION.md`): захват в движке, parallax, 4 пробы. **Не входит:** запекание в KTX2 «на лету» (только шаг 5, по желанию), динамическое обновление каждый кадр, пробы для прозрачных поверхностей.

## Суть
Сейчас зеркальная часть окружения берётся только из одной глобальной карты `t_prefiltered` (`pbr.wgsl:1041`, `ibl_enabled`). В комнате металл и вода отражают небо, а не стены.
Нужны локальные кубические пробы: коробка влияния, параллакс-коррекция (луч отражения пересекается с коробкой пробы, а не уходит в бесконечность), смешивание до двух проб по приоритету и расстоянию, переход в глобальный IBL там, где проб нет.
Цепочка отражений после этой задачи: **SSR (экран) → проба → глобальный IBL**. SSR (`post/ssr.rs`) уже подмешивается поверх итогового цвета, поэтому в самом `pbr.wgsl` достаточно заменить источник «prefiltered».

## Что уже есть (проверено чтением)
- `IBLProcessor` (`resource/ibl.rs`): `generate_prefiltered_map(device, queue, skybox_view) -> (Texture, TextureView)` — куб 128², 5 мипов, из `skybox_view` (куб); `generate_irradiance_map(…)` — куб 32². Обе принимают **любой** кубический вид, не только небо. `generate_cubemap` — из equirect. `export_ktx2` / `from_ktx2` — запись и чтение набора.
- `Renderer::render_frame_into(...)` (`renderer/frame.rs:61`) рисует в произвольный вид без окна; тесты используют `tests/common` (`renderer`, `target`). Камера задаётся `Camera` (`camera.rs`).
- Привязки PBR: группа 0 — `t_irradiance` (2), `t_prefiltered` (3), `t_brdf_lut` (4), `s_ibl` (5), `t_blue_noise` (6); группа 1 — до биндинга 22 (см. `pbr.wgsl:39–98`). Раскладки — `pass/shared_layouts.rs`, создание группы — `renderer/init.rs`, `renderer/settings.rs` (`set_ibl_resource` пересоздаёт группу 0).
- **Лимит:** устройство создаётся с `wgpu::Limits::default()` (`renderer/create.rs:77`): 16 сэмплируемых текстур на стадию. В `pbr.wgsl` их уже 12 (группа 0: 4, группа 1: 5, группа 2: 3). Куб-массив проб — 13-я, но GI-01…03 добавят ещё. **Шаг 1 поднимает лимит.**

## Шаг 1 — лимиты устройства и чистая математика (S)
1. `renderer/create.rs`: вместо `Limits::default()` собрать `required_limits` как `Limits::default()` с полями `max_sampled_textures_per_shader_stage`, `max_storage_buffers_per_shader_stage`, `max_storage_textures_per_shader_stage` = `min(adapter_limits.<поле>, 32 / 16 / 8)` (adapter_limits = `adapter.limits()`), но не меньше значений по умолчанию. Только Windows + Vulkan: на других устройствах остаётся минимум. Логировать итоговые значения (`log::info!`).
   Тест `tests/device_limits.rs` (GPU, пропуск без адаптера): `the_device_has_at_least_the_default_limits` (сравнить с `Limits::default()` по трём полям: `>=`).
2. Новый файл `src/reflection/mod.rs` + `src/reflection/probe.rs` (чистая математика, без GPU), `pub mod reflection;` в `lib.rs`:
```rust
/// Box-shaped reflection probe (world space).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReflectionProbe {
    /// Capture point.
    pub center: Vec3,
    /// Half-size of the influence box around `center`.
    pub half_extent: Vec3,
    /// Distance inside the box edge over which the weight falls from 1 to 0.
    pub blend_distance: f32,
    /// Larger wins when boxes overlap.
    pub priority: i32,
}
/// Direction after box (parallax) correction: the point where the ray from `pos` along `dir` leaves the box, minus the capture centre. `dir` is normalised.
pub fn parallax_direction(probe: &ReflectionProbe, pos: Vec3, dir: Vec3) -> Vec3;
/// Weight in [0,1]: 0 outside the box, 1 deeper than `blend_distance` from every face, linear in between.
pub fn probe_weight(probe: &ReflectionProbe, pos: Vec3) -> f32;
/// Picks at most two probes (highest priority, then highest weight) and normalises their weights so that they sum to `min(1, w0 + w1)`; the remainder goes to the global IBL.
pub fn select_probes(probes: &[ReflectionProbe], pos: Vec3) -> [(usize, f32); 2];   // (index, weight); weight 0 = unused slot
```
`parallax_direction`: пересечение луча с AABB `[center-half, center+half]` изнутри (`t = min over axes of (±half - local) / dir`), результат `(pos + dir*t) - center`; если `dir` по оси 0 — ось пропускается; если `pos` вне коробки — вернуть `dir` без изменений.
Тесты (юнит, в `probe.rs`):
```rust
#[test] fn the_parallax_direction_hits_the_wall_in_front() { /* box half 5, pos (0,0,0), dir +X -> (5,0,0) */ }
#[test] fn a_shifted_position_changes_the_corrected_direction() { /* pos (2,0,0), dir +X -> (5,0,0); pos (2,0,0), dir (0,1,0) -> (-2,5,0)... center-relative */ }
#[test] fn outside_the_box_the_direction_is_unchanged() { /* ... */ }
#[test] fn the_weight_is_zero_outside_one_inside_and_linear_in_the_blend_zone() { /* blend 1.0: at distance 0.5 from a face -> 0.5 */ }
#[test] fn the_higher_priority_probe_wins_when_boxes_overlap() { /* ... */ }
#[test] fn weights_of_two_probes_do_not_exceed_one() { /* ... */ }
#[test] fn no_probe_gives_zero_weights() { /* select_probes(&[], p) == [(0,0.0),(0,0.0)] */ }
```

## Шаг 2 — компонент ECS и хранилище проб (S)
- `ReflectionProbeComponent { probe: ReflectionProbe, resolution: u32 /*по умолчанию 128, степень двойки 32..=256*/, dirty: bool }` — в `redox_render::reflection` (derive `Reflect`, если у соседних компонентов он есть: **ПРОВЕРИТЬ** `grep -rn "derive(.*Reflect" crates/redox_render/src | head`).
- `ReflectionProbeSet` (в `Renderer`, поле `reflection: ReflectionProbeSet`): `MAX_PROBES = 4`, массив `[Option<ProbeSlot>; 4]`, `ProbeSlot { probe, prefiltered: Texture, view }`. Методы: `set_probes(&mut self, &[ReflectionProbe])` (отсекает лишние: оставляет по приоритету; возвращает число принятых), `probes()`, `uniform() -> ProbesGpu`.
- GPU-структура `ProbesGpu` (кратна 16): `count: u32`, `_pad[3]`, массив `[ProbeGpu; 4]` где `ProbeGpu { center_blend: [f32;4] /*xyz center, w blend_distance*/, half_extent_prio: [f32;4] }`. Тест размера: `probes_gpu_layout_is_16_aligned`.
- Куб-массив: одна `texture_cube_array<f32>` на 4 слоя (6 граней × 4 = 24 слоя), 128², 5 мипов, формат `Rgba16Float` (как у глобального prefiltered — **ПРОВЕРИТЬ** формат в `generate_prefiltered_map`). Пустые слоя заполнены нулями (вес 0 всё равно).
- Учёт памяти: категория текстур `MemoryCategory` (**ПРОВЕРИТЬ** `memory_budget.rs`, `renderer/memory.rs`), строка «reflection_probes».
- Тесты (GPU, `tests/reflection_probes.rs`): `set_probes_keeps_at_most_four_by_priority`; `the_probe_array_is_counted_in_the_memory_report`.

## Шаг 3 — захват пробы (M)
`Renderer::capture_reflection_probe(&mut self, slot: usize, objects: &[RenderObject], lights…)` — **или** отдельная функция в `reflection/capture.rs`, принимающая `&mut Renderer` (выбрать вариант с меньшим числом изменений в `Renderer`, записать выбор в отчёт):
1. Куб-цель `Rgba16Float` `resolution²×6` с `RENDER_ATTACHMENT | TEXTURE_BINDING | COPY_SRC`.
2. Для каждой из 6 граней: `Camera` в `center`, FOV 90°, aspect 1, направления `+X −X +Y −Y +Z −Z` (up по соглашению кубических карт: для ±Y — `∓Z`), рендер через `render_frame_into` в вид грани. Сам захват рисует **без** SSR/тумана/motion blur/DoF и без самих проб (защита от рекурсии: временно `reflection.enabled = false`), с небом (скайбокс включён).
3. `IBLProcessor::generate_prefiltered_map(device, queue, &cube_view)` → записать результат в слой слота (`copy_texture_to_texture`, 6 граней × 5 мипов).
4. `dirty = false`. Вызов не чаще раза за кадр; в `render_frame` проб, у которых `dirty`, **не** захватывается автоматически (v1: явный вызов игрой/редактором; автозахват при загрузке — в `PLAN_EXECUTION`, не здесь).
Тесты (GPU, `tests/reflection_probes.rs`):
```rust
#[test] fn a_captured_probe_contains_the_colour_of_the_wall_it_faces() { /* красная стена на +X от центра: при выборке prefiltered в направлении +X (читать мип 0 слоя 0 через copy в буфер) красный канал > 0.5, синий < 0.2 */ }
#[test] fn an_empty_scene_captures_the_sky_colour() { /* без объектов: цвет равен глобальному skybox ± допуск */ }
#[test] fn capture_restores_the_renderer_settings() { /* после захвата reflection.enabled/ssr/fog равны прежним */ }
```

## Шаг 4 — шейдер: проба вместо глобального prefiltered (M)
- `pbr.wgsl`: `@group(0) @binding(7) var t_probes: texture_cube_array<f32>;` и `@group(0) @binding(8) var<uniform> probes: ProbesGpu;` (**ПРОВЕРИТЬ** что 7 и 8 свободны в группе 0; иначе взять следующие и поправить `shared_layouts.rs`). Раскладка группы пересоздаётся в `renderer/settings.rs` (`set_ibl_resource`) и `init.rs`; добавить туда вид массива и буфер.
- Функция `probe_specular(world_pos, r, roughness) -> vec4<f32>` (rgb + вес): для i in 0..probes.count — `w = probe_weight(i, pos)` (зеркало Rust; формулы совпадают с шагом 1), направление `parallax_dir(i, pos, r)`, `textureSampleLevel(t_probes, s_ibl, dir, i, roughness*4.0)`; выбрать две лучшие по приоритету/весу, как `select_probes`. Результат: `mix(global_prefiltered, probe_color, total_weight)` — везде, где используется `prefiltered_color` (основной слой, `substrate_ibl` два лепестка, `coat_ibl_term`).
- Диффузная часть (`t_irradiance`) **не меняется** (GI-01 даст локальную диффузию).
- Флаг `reflection.enabled` (по умолчанию false, `Renderer::set_reflection_probes_enabled(bool)`); при `count == 0` или выключено — кадр **побитно** прежний (проверяется golden).
- Тесты (GPU): `a_glossy_floor_reflects_the_probe_wall_instead_of_the_sky` (пол металл roughness 0.1, проба захвачена в комнате с красной стеной: цвет пола рядом со стеной краснее, чем без пробы); `parallax_moves_the_reflection_with_the_viewer` (две позиции камеры вдоль стены: положение границы «стена/пол» в отражении сдвигается; без коррекции — нет); `outside_the_box_the_global_ibl_is_used` (объект вне коробки: кадр равен кадру без проб ± 1/255); `the_frame_is_unchanged_without_probes` (golden `golden_frames` и `ssr`-тесты зелёные без правок); `two_overlapping_probes_blend_smoothly` (на линии через зону смешивания яркость монотонна, нет скачка > 0.1).

## Шаг 5 — запекание в KTX2 (S, по желанию)
`redox_cook bake-probes` — **не писать, если шаги 1–4 заняли больше 7 дней**. Если делаем: `IblKtx2`/`export_ktx2` уже пишет набор; добавить загрузку `ReflectionProbeSet::load_baked(&[IblKtx2])` и тест `a_baked_probe_matches_the_runtime_capture` (±2/255).
Документ: абзац в `PLAN_LIGHTING.md` §3.2, `PLAN_EXECUTION.md` C4 → ✅ (или ◐), `CHANGELOG.md`.

## Границы
- **Читать:** `resource/ibl.rs`, `renderer/{frame,settings,init,create}.rs`, `pass/shared_layouts.rs`, `shaders/pbr.wgsl` (блоки IBL: строки 1030–1090 и `coat_ibl_term`), `post/ssr.rs` (порядок цепочки), `tests/common/mod.rs`, `tests/ktx2_cube_ibl.rs` (как читают куб).
- **Создать:** `src/reflection/{mod,probe,capture}.rs`, `tests/reflection_probes.rs`, `tests/device_limits.rs`.
- **Менять:** `renderer/create.rs` (лимиты), `renderer/{mod,init,settings}.rs`, `pass/shared_layouts.rs`, `shaders/pbr.wgsl`, `lib.rs`, `shader/mod.rs` (если нужно).
- **Не трогать:** `post/ssr.rs` и `ssr.wgsl`, VG-шейдеры (`vg_resolve.wgsl` берёт IBL отдельно — перевод на пробы в MAT-07/после), эталонные кадры.

## Проверка
`cargo fmt`, `cargo clippy -p redox_render --all-targets -- -D warnings`, `cargo test -p redox_render --test reflection_probes --test device_limits --test golden_frames --test post_fx`, `cargo check --workspace --all-targets`, `python scripts/check_file_sizes.py` (`pbr.wgsl` уже большой — новые функции вынести в `shaders/reflection_probe.wgsl`, если подключение общих кусков позволяет: **ПРОВЕРИТЬ** `src/shader/mod.rs`, как собираются WGSL-модули).

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Шаги строго по порядку, каждый — отдельный коммит. Любая смена раскладки привязок — одним коммитом с зеркальной правкой WGSL и тестом размера. Если лимит текстур не поднимается (адаптер не поддерживает 20+) — остановись и сообщи значение `adapter.limits()`.

## Закрытие (финальная волна)

Исход обязателен: ✅ / ⛔ / 📌 (`../FINAL_WAVE.md` §1). Агенту разрешён `cargo check -p <крейт> --all-targets` в своём worktree; тесты, clippy и замеры — у координатора. Шаг, у которого приёмочный тест после доработки красный, не остаётся «частично»: агент сообщает измеренное число, исход назначает координатор.

- **Осталось:** (1) детерминированный тай-брейк выбора ≤ 2 проб: приоритет, затем вес, затем индекс пробы; тест `three_probes_with_equal_priority_and_weight_pick_the_lowest_indices` и тест «порядок вставки не меняет выбор»; (2) HDR-захват граней: одна попытка (линейный `Rgba16Float` выход граней минуя пост-цепочку); если пост-цепочка пишет в формат поверхности и правка не локальна — 📌 «захват в LDR» в `KNOWN_LIMITS.md`; (3) шаг 5 (KTX2): проверить, что `ktx2_cube_ibl` покрывает загрузку проб с диска, иначе дописать тест.
- **Приёмка:** `reflection_probes` зелёный, побитная нейтральность захвата сохранена.
