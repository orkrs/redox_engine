# MAT-08 — Материалы листвы: двусторонние, режимы альфы, просвечивание, вершинные цвета

- **Статус:** не начат.
- **Размер:** 5 шагов (S, M, M, M, S) ≈ 5–7 дней. **Крейт:** `redox_render`. **Зависит от:** ничего (шаг 5 зависит от INV-01). **Блокирует:** FOL-03.
- **Модель:** haiku по шагам, после каждого проверка координатора. Один писатель рендерера одновременно (`../FOREST_WAVE.md` §3).
- Источник: `../FOREST_SCENE_FINDINGS.md`, E5, E6, E7; план 5.3.

## Суть
Сейчас прямой проход режет задние грани, листва и трава видны с одной стороны (обход: дублирование треугольников при загрузке в примере `forest`). Прозрачность только стохастическая с порогом 0,5 для теней и без сохранения покрытия в mip. Нет просвечивания листвы на солнце. Нужно: двусторонние материалы, нормальные режимы альфы, тонкая трансмиссия, при необходимости вершинные цвета.

## Что уже есть (проверено чтением)
- Режим отсечения задан в: `pass/pbr.rs:51` (Back), `pass/normal.rs:94` (Back), `pass/velocity.rs:257` (Back), `pass/vg_resolve.rs:100,570` (Back), `shadow/depth_pass.rs:101` (Back), `matgraph_passes.rs:348,441,757` (Back) и `388,801` (None); `shadow/masked.rs:53,101` уже None. **ПРОВЕРИТЬ** список командой `grep -rn cull_mode crates/redox_render/src`.
- `material.rs`: `Material { alpha_test: bool, subsurface: Option<SubsurfaceProfile>, … }`, флаги `material_flags` (биты 0–8 заняты: ALBEDO, NORMAL, MR, SUBSTRATE, ALPHA_TEST=16, COAT, ANISOTROPY, SUBSURFACE, REFRACTION), `MaterialUniform`.
- `shaders/material_surface.wgsl`: `material_surface` делает `discard` по `alpha < tex.dither` (stochastic alpha); `shaders/shadow_masked.wgsl`, `vsm_depth_masked.wgsl`: порог 0,5 зашит.
- Группировка вызовов по `(mesh, material)` в `renderer/batching.rs`.

## Шаг 1 — расследование состояния (S)
- Отчёт `../notes/MAT-08-step1.md`: список всех мест выбора отсечения (с файлом и строкой), какие из них зависят от материала, какие ключи кэша пайплайнов затронуты (`shader/` — **ПРОВЕРИТЬ** `ShaderKey`, `Features`), как `vg_resolve` обходит материал.
- Решение с обоснованием: вариант A (пайплайн на `double_sided`) или вариант B (всегда `cull None` + переворот нормали по `@builtin(front_facing)`). Критерий: меньше кода и ни одного изменения кадра для односторонних материалов. Исполнитель предлагает, координатор утверждает до шага 2.

## Шаг 2 — двусторонние материалы (M; нужен шаг 1)
- `material.rs`: `double_sided: bool`, бит 9 `DOUBLE_SIDED` в `material_flags`; `renderer/assets.rs` (`add_material`, `build_material_bind_group`) передаёт флаг в `MaterialUniform.flags`.
- Шейдеры: `material_surface.wgsl`/`pbr.wgsl` — для задней грани нормаль и тангент меняют знак; `normal.wgsl` (префасс) и `object_velocity.wgsl` — по выбранному варианту.
- Тесты (GPU, по образцу `tests/mat06_gpu.rs`, отдельный бинарь `tests/mat08_gpu.rs`): `a_double_sided_quad_is_lit_the_same_from_both_sides` (разница яркости ≤ 3/255 при симметричном свете), `a_single_sided_quad_is_invisible_from_behind`, `single_sided_frames_are_bitwise_unchanged` (существующие golden).
- После шага обновить загрузчик в `examples/forest/model.rs`: убрать дублирование треугольников, ставить `double_sided` для `alphaMode != OPAQUE`.

## Шаг 3 — режимы альфы и mip с сохранением покрытия (M)
- `material.rs`: `AlphaMode::{Opaque, Mask(f32), Dither}` вместо `alpha_test: bool` (оставить `masked()` и `alpha_test` как совместимую обёртку; **ПРОВЕРИТЬ** все использования: `grep -rn alpha_test crates`). Порог передаётся в униформу; `shadow_masked.wgsl` и `vsm_depth_masked.wgsl` читают порог материала вместо 0,5.
- `resource/texture.rs` (**ПРОВЕРИТЬ** где строятся mip): для текстур материалов с `Mask` масштабировать альфу каждого mip так, чтобы доля пикселей выше порога равнялась доле в mip 0 (классический метод Castano/Iñigo Quilez).
- Тесты: `coverage_is_preserved_across_mips` (процентная доля покрытия mip 4 против mip 0: разница ≤ 5 %), `a_mask_cutoff_changes_the_silhouette` (порог 0,2 против 0,8 на градиентной текстуре), `shadow_cutoff_follows_the_material` (тень вырезной карточки зависит от порога материала).

## Шаг 4 — просвечивание листвы (M)
- `Material`: `transmission: f32` (0…1) и `transmission_tint: Vec3`; флаг `HAS_TRANSMISSION` (бит 10).
- `pbr.wgsl` (прямой свет направленного света и кластерных точечных/прожекторных): `trans = albedo * transmission_tint * transmission * saturate(dot(-n, l)) * shadow`, добавляется к диффузии; IBL: освещённость по инвертированной нормали с тем же весом. Сначала проверить, можно ли переиспользовать `SubsurfaceProfile.wrap`; решение записать в отчёт.
- Тесты (GPU): `a_leaf_is_brighter_when_the_sun_is_behind_it` (карточка с `transmission = 0.6`, солнце сзади даёт яркость ≥ 1,3 × яркости при солнце спереди в тени), `a_material_without_transmission_is_bitwise_unchanged`.

## Шаг 5 — вершинные цвета (S; только если INV-01 подтвердит H2 или решение координатора)
- Вариант без смены раскладки `Vertex`: отдельный поток `color: [u8;4]` (**ПРОВЕРИТЬ**, как VG и скиннинг читают вершины: `vertex_compressed.wgsl`); использовать как запечённый AO (умножение albedo/ambient) и тонировку.
- Оценка стоимости записывается в отчёт; если затрагивается VG или скиннинг, шаг получает исход 📌 с описанием.
- Тесты: `vertex_colour_multiplies_the_albedo`, `a_mesh_without_vertex_colour_is_bitwise_unchanged`.

## Границы
- **Читать:** перечисленные проходы и шейдеры, `renderer/{assets,batching}.rs`, `resource/texture.rs`, `tests/mat06_gpu.rs`.
- **Создать:** `tests/mat08_gpu.rs`, `docs/engineering/notes/MAT-08-step1.md`.
- **Менять:** `material.rs`, `renderer/assets.rs`, `shader/*` (ключи), шейдеры, `resource/texture.rs`, `examples/forest/model.rs`.
- **Не трогать:** GI, VG (кроме читаемой флага), туман, небо.

## Проверка
`cargo fmt`, `cargo check -p redox_render --all-targets`; свои тесты по одному; существующие `mat06_gpu`, `golden_frames` перед сдачей шага. Clippy и полный прогон — координатор.

## Для исполнителя
См. `../FOREST_WAVE.md` §4. Шаг за заход. Любое изменение кадров без новых флагов материала считается ошибкой: тесты «побитно прежние» обязательны в каждом шаге.

## Закрытие
✅ / ⛔ / 📌 по шагам. Обход E5 в примере убирается в том же шаге 2.
