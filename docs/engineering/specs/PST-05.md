# PST-05 — Отражения в экранном пространстве (SSR, C3)

- **Размер:** M + M + S (три шага). **Крейт:** `redox_render`. **Зависит от:** PST-01 (каркас `PostFx`, готово). **Блокирует:** ничего.
- **Модель:** haiku. Файлы `post/post_fx.rs`, `renderer/{mod,init,resize,settings}.rs`, `renderer/frame/post.rs` — одному исполнителю (как в PST-01/02). `normal.wgsl` и `pass/normal.rs` — тому же.
- **По умолчанию выключено; выключенное не меняет кадр побитно.**

## Суть и решение по данным (шаг 0, принято при написании спеки)
Проблема из плана: в forward-рендерере нет буфера шероховатости, а SSR без него даёт плохую картинку (одинаково «зеркальные» все поверхности).
Проверено чтением кода:
- Есть нормальный префасс `pass/normal.rs` + `shaders/normal.wgsl`: пишет `vec4(world_normal, linear_depth)` в `Rgba16Float` (`renderer/init.rs:53,305–311`, переменные `normal_texture/normal_view`). `linear_depth = view_z / 1000` (`FAR`, `normal.wgsl`). То есть **нормали и линейная глубина для SSR уже есть**, hi-z не обязателен.
- Шейдер префасса не знает материала: он читает только `models[instance]` (`ModelUniform`, 160 байт: `model, normal_matrix, color, bounds_min, bounds_extent`, `pass/shared_layouts.rs:79–93`; заполняется в `renderer/scene_sync.rs:46–76`, где под рукой есть `obj.material_index`).
- Скалярные `roughness`/`metallic` материала лежат на CPU в `Material` (`material.rs:9–21`); карты roughness/metallic (`mr_texture_index`) префасс не прочтёт.

**Выбранный вариант (один):** второй цветовой вывод префасса `Rg8Unorm` = (roughness, metallic), значения **скалярные из материала объекта**, передаются через `ModelUniform`.
Обоснование: минимум изменений (один атрибут вывода + два float), не трогает основной проход PBR/MSAA; карты roughness игнорируются сознательно («оптимально, не идеально»): SSR включается только для поверхностей с `roughness < 0.5`, а у них скаляр почти всегда задаёт общий уровень.
Отвергнуты: G-буфер (смена архитектуры), SSR только по флагу материала (не даёт плавного перехода), чтение `Material` из шейдера SSR по ID (нужен ID-буфер и биндинг массива материалов).

## Шаги
### Шаг 1 — данные шероховатости (M)
1. `ModelUniform`: использовать свободные `bounds_min[3]` и `bounds_extent[3]` как `roughness`/`metallic`. **ПРОВЕРИТЬ:** `grep -n "bounds_min.w\|bounds_extent.w" shaders/*.wgsl` и функцию `bounds_vectors` (`grep -rn "fn bounds_vectors" src`) — если `.w` где-то используются, добавить явные поля `surface: [f32; 4]` (160 → 176 байт) и изменить **все** `struct Model` в WGSL (`grep -n "struct Model" shaders/*.wgsl`, их не меньше пяти: порядок полей обязан совпасть).
2. `update_model_buffer` (`scene_sync.rs:46`): значения из `Material` по `obj.material_index`. Нужен вектор `material_surface: Vec<[f32; 2]>` в `Renderer`, заполняемый в `add_material` (рядом с `material_blend_flags`, **ПРОВЕРИТЬ** `grep -n material_blend_flags src/renderer/*.rs`).
3. `NormalPass::new` (`pass/normal.rs`): второй target `Rg8Unorm`; `fs_main` возвращает структуру `{ @location(0) normal_depth, @location(1) surface }`. Новая текстура `surface_texture/surface_view` в `Renderer`, пересоздание в `resize.rs` рядом с `normal_texture`.
4. VG-путь и скиннинг, пишущие в `normal_texture` другими пассами (`grep -rn "normal_view" src`), писать `surface = (1.0, 0.0)` (шероховатая = SSR выключен для них) — ничего не должно остаться без второго target (иначе ошибка валидации wgpu).
Тесты: GPU-тест `tests/ssr.rs::surface_target_holds_the_material_roughness` — два куба с roughness 0.1 и 0.9, чтение `surface_texture` (как `tests/object_velocity.rs` читает `taa_velocity_tex`): пиксели куба дают ≈ (0.1, m) и ≈ (0.9, m) с допуском 1/255·2. `golden_frames` не меняются.

### Шаг 2 — чистая математика и трассировка (M)
Файл `post/ssr.rs` (формулы; зеркало в WGSL, «держать в синхроне» как в `dof.wgsl`):
```rust
/// Parameters of the screen-space reflections.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SsrSettings {
    /// Reflections are drawn only on surfaces rougher-than this are skipped (0..1). Default 0.5.
    pub max_roughness: f32,
    /// Longest ray in view-space units. Default 30.0.
    pub max_distance: f32,
    /// Steps of the linear march (the hit is then refined by 5 bisection steps). Default 32.
    pub steps: u32,
    /// Depth thickness (view-space units) below which a ray behind a surface still counts as a hit. Default 0.3.
    pub thickness: f32,
    /// Overall strength 0..1. Default 1.0.
    pub intensity: f32,
}
/// 0 at `max_roughness` and above, 1 for a mirror; smooth in between: (1 - r / max)^2 clamped.
pub fn roughness_weight(roughness: f32, max_roughness: f32) -> f32;
/// Fade towards the screen edges: 1 inside the central 80 %, 0 at the border, for uv in 0..1.
pub fn edge_fade(uv: [f32; 2]) -> f32;
/// Schlick Fresnel with F0 = mix(0.04, 1, metallic).
pub fn fresnel_weight(n_dot_v: f32, metallic: f32) -> f32;
/// Final blend weight: roughness_weight * edge_fade(hit_uv) * fresnel_weight * intensity, clamped to 0..=0.9.
pub fn ssr_weight(roughness: f32, metallic: f32, n_dot_v: f32, hit_uv: [f32; 2], s: &SsrSettings) -> f32;
```
Шейдер `shaders/ssr.wgsl`: полноэкранный треугольник (шаблон `dof.wgsl`), входы — цвет (после TAA, ping-pong `PostFx`), нормаль+линейная глубина (`normal_view`), `surface_view`, uniform `{ view: mat4, proj: mat4, inv_proj: mat4, screen, near, far, SsrSettings… }`.
Алгоритм: восстановить позицию вида из UV и `linear_depth*1000` (учесть: `linear_depth = view_z/FAR`); отражённый луч `reflect(-v, n)` в пространстве вида; шаг `max_distance/steps`; на каждом шаге проекция точки в UV, сравнение глубины луча с глубиной сцены (`normal_view.a * 1000`), попадание, если `0 < ray_z - scene_z < thickness`; затем 5 шагов бисекции; цвет в точке попадания читается из входного цвета; `weight = ssr_weight(...)`; результат `mix(color, hit_color, weight)`; если попадания нет — `color` без изменений.
Известное упрощение (записать в doc-комментарии): спекуляр окружения из PBR уже есть в цвете, поэтому смешивание заменяет его частично, а не складывается — потеря энергии допустима; настоящее разделение спекуляра — вне спеки.
Подключение: новый эффект в `PostFx::run_chain` **первым** в цепочке (до motion blur и DoF), `Renderer::set_ssr(Option<SsrSettings>)` / `ssr()`; `None` по умолчанию. Нужны матрицы вида/проекции: **ПРОВЕРИТЬ** где `Renderer` хранит `view` (`grep -n "inv_proj" src/renderer/*.rs`); `CameraUniform` содержит `view_proj` и `inv_proj`, но не `view` — view = `inv_proj * view_proj`... проще передать отдельно из `Camera`+позы (как в `update_camera_buffer`).

Юнит-тесты (`post/ssr.rs`):
```rust
#[test] fn a_mirror_gets_the_full_roughness_weight_and_a_rough_surface_none() { assert_eq!(roughness_weight(0.0, 0.5), 1.0); assert_eq!(roughness_weight(0.5, 0.5), 0.0); assert_eq!(roughness_weight(0.9, 0.5), 0.0); }
#[test] fn the_edge_fade_is_one_in_the_middle_and_zero_at_the_border() { assert_eq!(edge_fade([0.5, 0.5]), 1.0); assert_eq!(edge_fade([0.0, 0.5]), 0.0); assert_eq!(edge_fade([1.0, 1.0]), 0.0); }
#[test] fn fresnel_grows_at_grazing_angles_and_metal_is_always_strong() { assert!(fresnel_weight(0.05, 0.0) > fresnel_weight(1.0, 0.0)); assert!(fresnel_weight(1.0, 1.0) > 0.9); }
#[test] fn the_final_weight_is_capped() { let s = SsrSettings::default(); assert!(ssr_weight(0.0, 1.0, 0.0, [0.5, 0.5], &s) <= 0.9); }
#[test] fn nan_inputs_give_zero_weight() { /* ssr_weight(NAN, 0, 1, [0.5,0.5], &s) == 0.0 */ }
```
GPU-тесты (`tests/ssr.rs`): (а) `with_ssr_off_the_frame_is_unchanged` (два рендера побитно равны, в том числе после `set_ssr(None)`); (б) `a_mirror_floor_reflects_a_bright_cube`: пол roughness 0.05 + яркий куб над ним; в пикселях пола под кубом цвет заметно ярче/ближе к цвету куба, чем с выключенным SSR (разница > 0.1 по каналу куба); (в) `a_rough_floor_is_not_changed` (roughness 0.9 → кадр с SSR и без побитно равны); (г) `ssr_survives_a_resize`.

### Шаг 3 — отладка и документация (S)
Режим отладки (`debug_view`) не нужен; в `docs/` добавить абзац: как включить `set_ssr`, ограничения (нет отражений того, чего нет на экране, нет карт roughness, потеря энергии спекуляра). Живая проверка владельцем: лужа/глянцевый пол в сцене.

## Границы
- **Читать:** `shaders/normal.wgsl`, `pass/normal.rs`, `renderer/init.rs:40–60,295–350`, `renderer/scene_sync.rs:46–76`, `pass/shared_layouts.rs:75–95`, `material.rs`, `post/post_fx.rs` (весь), `shaders/dof.wgsl`, `renderer/frame/post.rs`, `tests/post_fx.rs`, `tests/object_velocity.rs`.
- **Создать:** `post/ssr.rs`, `shaders/ssr.wgsl`, `tests/ssr.rs`.
- **Менять:** `pass/normal.rs`, `shaders/normal.wgsl`, `pass/shared_layouts.rs`, `renderer/scene_sync.rs`, `renderer/{mod,init,resize,settings}.rs`, `post/post_fx.rs`, `renderer/frame/post.rs`. Шейдеры с `struct Model`, если пришлось расширить структуру.
- **Не трогать:** `pbr.wgsl` (кроме `struct Model` при расширении), TAA, тени.

## Проверка
`cargo fmt`, `cargo clippy -p redox_render --all-targets -- -D warnings`, `cargo test -p redox_render --lib ssr`, `cargo test -p redox_render --test ssr --test golden_frames --test post_fx --test resize_bloom --test object_velocity`, `python scripts/check_file_sizes.py` (`post_fx.rs` уже 573+ строк — новый эффект сделать в `post/ssr.rs`, а в `post_fx.rs` оставить только вызов; если `post_fx.rs` > 800, вынести DoF/blur в `post/passes.rs`).
Стоимость (в отчёт): время кадра с SSR и без — через `FrameCpuTimings` (CPU) и, если доступен, GPU-профайлер; цель ≤ 0,6 мс на 1080p.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Шаг 1 — отдельный коммит, зелёный на всех существующих тестах **до** шага 2. Golden-эталоны не обновлять.
