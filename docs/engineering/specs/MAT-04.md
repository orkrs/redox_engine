# MAT-04 — Материалы, E4: Substrate-лайт — слои с сохранением энергии, clearcoat, анизотропия

- **Статус:** выполнены шаги 1–4; шаг 5 (узлы графа) ждёт MAT-02.

- **Размер:** 5 шагов (S, M, M, S, S) ≈ 8 дней. **Крейты:** `redox_render` (+ `redox_matgraph` на шаге 5). **Зависит от:** MAT-01 (`SurfaceData`). Граф (MAT-02) для шагов 1–4 **не нужен**. **Блокирует:** MAT-07.
- **Модель:** haiku по шагам 1, 4, 5; шаги 2–3 (формулы BRDF) — haiku **с проверкой координатора после каждого**.
- Объём — «минимум» плана E4: slab + Vertical/Horizontal, до 2 слоёв. **Не входит:** 3+ слоёв, thin film.

## Суть и аудит
Аудит `pbr.wgsl` и `material.rs` показал, что «Substrate» уже частично есть, но это **взвешенная сумма лепестков**, а не слои:
- `Material.layers: Option<[MaterialLayer; 2]>` (цвет, металличность, шероховатость, вес), `sheen`, флаг `HAS_SUBSTRATE`, WGSL `eval_substrate_lit` (`pbr.wgsl:635`), `eval_sheen_lobe` (616), IBL-вклад слоёв (`pbr.wgsl:~929–975`). Сумма `w0 + w1` отнимается от веса базы (`w_base = max(1 - w0 - w1, 0)`).
- Нет: покрытия (clearcoat) со своей нормалью и ослаблением нижнего слоя, закона сохранения энергии при вертикальном наложении, анизотропии, горизонтального смешивания по маске.
Цель шагов — настоящие `Vertical(coat, base)` и `Horizontal(a, b, mask)`, совместимые со старым `Material` (миграция старого = частный случай, кадр **не меняется** для материалов без новых полей).

## Шаг 1 — модель на CPU и тест энергии (S, чистая математика)
Файл `redox_render/src/substrate.rs` — эталон формул на Rust (WGSL повторяет):
```rust
pub struct Slab { pub base_color: [f32; 3], pub metallic: f32, pub roughness: f32, pub anisotropy: f32, pub f0: f32 /* 0.04 */ }
pub struct Coat { pub weight: f32, pub roughness: f32, pub ior: f32 /* 1.5 */, pub tint: [f32; 3] }
/// Fresnel reflectance (Schlick) of the coat at cosine `n_dot_v`.
pub fn coat_fresnel(coat: &Coat, n_dot_v: f32) -> f32;
/// Energy left for the layer below after the coat: `1 - weight * coat_fresnel` times the tint transmittance.
pub fn transmitted_through_coat(coat: &Coat, n_dot_v: f32) -> [f32; 3];
/// Directional albedo bound of a slab (<= 1 for any parameters), used by the energy tests: numerical integration over the hemisphere.
pub fn slab_albedo(slab: &Slab, n_dot_v: f32, samples: u32) -> f32;
/// Total albedo of Vertical(coat, slab).
pub fn vertical_albedo(coat: &Coat, slab: &Slab, n_dot_v: f32, samples: u32) -> f32;
```
Тесты (энергосбережение — главное):
```rust
#[test] fn a_slab_never_reflects_more_than_it_receives() { /* перебор: roughness 0.05..1 (шаг 0.15), metallic 0/0.5/1, n_dot_v 0.05..1 → slab_albedo <= 1.0 + 1e-3 */ }
#[test] fn a_coated_slab_never_reflects_more_than_it_receives() { /* + coat.weight 0..1, roughness 0.05..1, ior 1.3..2 → vertical_albedo <= 1.0 + 1e-3 */ }
#[test] fn a_zero_weight_coat_is_invisible() { /* vertical_albedo(coat weight 0) == slab_albedo ± 1e-6 */ }
#[test] fn the_coat_darkens_the_base_at_normal_incidence_by_its_fresnel() { /* transmitted ≈ 1 - 0.04 */ }
#[test] fn fresnel_grows_towards_grazing_angles() { /* ... */ }
#[test] fn extreme_roughness_values_are_finite() { /* 0.0, 1e-6, 1.0 → не NaN/inf */ }
```

## Шаг 2 — clearcoat в WGSL (M)
- `SurfaceData` (MAT-01) получает `coat_weight, coat_roughness, coat_ior, coat_tint: vec3, coat_normal: vec3`. По умолчанию `coat_weight = 0` → ветка пропускается, картинка прежняя (побитно).
- В `fs_main`: `lit = coat_spec(...) + transmitted_through_coat * lit_base` (формулы шага 1; GGX/Smith/Schlick — готовые функции `distribution_ggx`, `geometry_smith`, `fresnel_schlick`, `pbr.wgsl:553–585`). IBL: добавить покрытие к `specular_ibl` (`prefiltered` на `coat_roughness`, `env_brdf` по `coat`), нижнему слою — множитель `transmitted`.
- Свет: каждый источник даёт `coat_spec`; сохранять порядок существующих операций в ветке `coat_weight == 0`.
- `MaterialUniform` (CPU `material.rs:38`) + структура WGSL: поля покрытия добавить в **конец** (смена раскладки → обновить `size_of`-тест; **ПРОВЕРИТЬ** есть ли тест размера uniform: `grep -rn "size_of::<MaterialUniform" crates`).
- `Material::coat(weight, roughness)` builder (`material.rs`, по образцу `metallic()/roughness()` стр. 106–115).
- Тесты (GPU, `tests/substrate.rs`): `a_material_without_a_coat_renders_as_before` (golden побитно); `a_coat_adds_a_second_highlight` (шар с `roughness 0.6` основа + coat `roughness 0.05`: на кадре найдётся узкий яркий пик над широким бликом — метрика: максимум яркости кадра растёт > 20% при coat 1.0 vs 0.0 при том же свете); `the_coat_darkens_the_base_diffuse_slightly` (средняя яркость диффузной области с coat ниже, чем без, но > 90%); `no_pixel_exceeds_the_energy_budget` (белый диффузный шар под единичным IBL-освещением: максимум яркости с coat ≤ без coat + порог спекуляра ≤ 1.0 после тон-маппинга… — использовать HDR-цель, если тест-харнесс позволяет; иначе пропустить с `#[ignore]` и записать причину).

## Шаг 3 — анизотропия (M)
- `SurfaceData`: `anisotropy: f32` (−1..1), `tangent: vec3`. Тангенты в основном проходе: вершинный формат `CompressedVertex` уже содержит тангент (волна 6, `skin_compute.wgsl` октаэдр + знак), **ПРОВЕРИТЬ** `vertex_compressed.wgsl` — доступен ли тангент в `fs_main`; если нет — вывести из `dpdx/dpdy` (запасной вариант, записать в отчёт).
- Анизотропный GGX (Burley): `D_aniso`, `V_aniso`; растяжение: `ax = roughness² * (1 + anisotropy)`, `ay = roughness² * (1 - anisotropy)`. При `anisotropy == 0` — прежний код (ветка).
- Эталон формул `d_ggx_aniso(...)` — в `substrate.rs` на Rust, тест соответствия WGSL через compute (допуск 1e-3).
- Тесты: `zero_anisotropy_equals_isotropic_ggx` (юнит, Rust: `d_ggx_aniso(a=0) == d_ggx` ± 1e-6); `anisotropic_d_integrates_to_about_one` (численно по полусфере ±5%); GPU `an_anisotropic_highlight_is_stretched` (шар/цилиндр: ширина блика вдоль тангента больше, чем поперёк, в ≥ 1.5 раза при `anisotropy=0.8`).

## Шаг 4 — Horizontal(a, b, mask) и совместимость (S)
- `Horizontal`: два набора параметров слэба и маска 0..1 → линейная смесь **параметров** (не результатов освещения), кроме случая, когда `metallic` различается сильно — тогда смесь результатов (`abs(a.metallic - b.metallic) > 0.5`), иначе дорого вдвое. Флаг/ветка: смесь параметров для простоты в v1; смесь результатов — вне объёма (записать как ограничение).
- Миграция: `Material` → `Slab` (одна функция `fn slab_from_material(&Material) -> Slab` в `substrate.rs`); два слоя `MaterialLayer` остаются как есть (старый путь `eval_substrate_lit`), новый `coat` — поверх. Документ о совместимости в `material.rs`.
- Тесты: `mix_with_mask_zero_equals_a` / `...one_equals_b` (юнит над параметрами: `mix_slabs(a, b, 0.0) == a`); `migration_keeps_the_legacy_parameters` (`slab_from_material` для `Material::solid(c).metallic(0.3).roughness(0.4)` → поля равны); `golden_frames` побитно.

## Шаг 5 — граф: узлы Substrate (S, после MAT-02)
`NodeKind::{Slab, Coat, Vertical, Horizontal}` и выход графа `SurfaceOutput` принимает один `Slab`-значение; генерация заполняет поля `SurfaceData` (шаг 2–3). Тесты — как в MAT-02 (валидация, naga, интерпретатор для чисто числовых полей); `a_graph_with_a_coat_matches_the_builder_material` (GPU: граф coat 1.0 vs `Material::coat` — кадры в допуске 2/255).

## Границы
- **Читать:** `pbr.wgsl:553–670,905–1000`, `material.rs`, `vertex_compressed.wgsl`, `PLAN_MATERIALS.md` §2, MAT-01.
- **Создать:** `redox_render/src/substrate.rs`, `tests/substrate.rs`.
- **Менять:** `pbr.wgsl` / `material_surface.wgsl`, `material.rs`, `vg_resolve.wgsl` (общая функция — MAT-01), `redox_matgraph` (шаг 5).
- **Не трогать:** тени, GI, SSS.

## Проверка
`cargo fmt`, `cargo clippy -p redox_render --all-targets -- -D warnings`, `cargo test -p redox_render --lib substrate`, `cargo test -p redox_render --test substrate --test golden_frames --test material_textures`, `python scripts/check_file_sizes.py`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Шаг 1 (энергия) должен быть зелёным до любых изменений шейдера. Никаких правок формул старого пути (`eval_substrate_lit`) — картинка прежних материалов побитно прежняя.
