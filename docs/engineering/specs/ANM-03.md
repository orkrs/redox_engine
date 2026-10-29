# ANM-03 — Морфы на GPU (compute-скиннинг с блендшейпами)

- **Размер:** M. **Крейт:** `redox_render`. **Зависит от:** ANM-02 (данные, импорт, `deform_morphed_into` — эталон на CPU). **Блокирует:** ничего.
- **Модель:** haiku. Файлы `skinning_gpu.rs`, `shaders/skin_compute.wgsl`, `skinning.rs` (render) — одному исполнителю.
- **Условие:** ANM-02 принят в `main`. Без него остановиться.

## Суть
GPU-скиннинг (`skinning_gpu.rs`, `shaders/skin_compute.wgsl`) повторяет `SkinnedGeometry::deform_into` + квантование вершины в 16-байтный `CompressedVertex`. Морфы по ANM-02 считаются на CPU (`deform_morphed_into`), в GPU-пути игнорируются.
Нужно, чтобы GPU-путь делал то же: к позиции и нормали (и тангенту) bind-позы добавляются взвешенные смещения целей **до** блендинга матриц, результат совпадает с CPU-эталоном в пределах квантования.

## Что уже есть (проверено чтением)
- `GpuSkinVertex` (80 байт, `skinning_gpu.rs:19`), `GpuSkinParams` (36), `SkinPipeline` (45; layout: binding 0 uniform params, 1 bind-вершины, 2 матрицы кожи, 3 выход `vec4<u32>`), `GpuSkin::new/upload/dispatch/run`, `run_batch` (247).
- Шейдер: `Params { bounds_min, bounds_extent, vertex_count, matrix_count, _pad0, _pad1 }`; `blend_matrix`; октаэдрическое кодирование нормали/тангента, знак ручности (как в `compress_one`).
- Тесты `tests/gpu_skinning.rs` сравнивают GPU и CPU (с тангентами, ручность — см. волну 6).

## Решение
Плотный буфер смещений (просто, без разреженности; лица ~50 целей × 10k вершин × 16 Б = 8 МБ — приемлемо):
```text
@group(0) @binding(4) var<storage, read> morph_deltas  : array<vec4<f32>>;  // [target * vertex_count + vertex].xyz = position delta
@group(0) @binding(5) var<storage, read> morph_normals : array<vec4<f32>>;  // same layout, xyz = normal delta (zeros when the target has none)
@group(0) @binding(6) var<storage, read> morph_weights : array<f32>;        // [target]
```
`Params`: заменить `_pad0` на `morph_count: u32` (раскладка WGSL ↔ `GpuSkinParams` обязана совпасть; **ПРОВЕРИТЬ** выравнивание, структура 80 байт сейчас).
В шейдере перед `blend_matrix`: `var p = bind.position; var n = bind.normal; for (t in 0..morph_count) { let w = morph_weights[t]; if (w != 0.0) { p += w * morph_deltas[t*vertex_count+v].xyz; n += w * morph_normals[...].xyz; } }`, затем `n = normalize(n)` (если длина > 0) и дальше прежний код.
Тангенты: как в CPU-эталоне ANM-02 (если шаг 1 ANM-02 морфит тангенты — добавить `morph_tangents`; иначе тангенты не морфятся, согласовать с CPU: **ПРОВЕРИТЬ** в `morph_into`).
Без целей: привязать пустые «заглушки» по 16 байт к 4/5/6 и `morph_count = 0` — вывод **побитно** прежний (тест).
Загрузка: `GpuSkin::new` принимает `&SkinnedGeometry` (уже принимает) и заполняет буферы смещений из `morph_targets`; `GpuSkin::upload_morph_weights(&self, queue, &[f32]) -> bool` (длина ≤ числу целей, лишнее игнорируется, недостающее — 0).
Предел целей: `MAX_MORPH_TARGETS = 128`; больше — `GpuSkin::new` возвращает `None`/ошибку (как для недопустимой геометрии сейчас — **ПРОВЕРИТЬ** тип результата) и вызывающий откатывается на CPU.

## Тесты (`tests/gpu_skinning.rs`, пропуск без адаптера; геометрия строится кодом, не из файла)
```rust
#[test] fn gpu_morphs_match_the_cpu_reference() {
    // 3 вершины, 2 цели, веса [0.5, 0.25], кость повёрнута; GPU-вывод разжать (как существующий тест) и сравнить с deform_morphed_into
    // допуск — шаг квантования позиции (bounds_extent / 65535 * 2) и нормали (2/127)
}
#[test] fn zero_morph_weights_equal_the_unmorphed_output() { /* побитно с выводом геометрии без целей */ }
#[test] fn a_model_without_targets_is_unchanged_bit_for_bit() { /* прежний тест GPU↔CPU остаётся зелёным без правок */ }
#[test] fn updating_the_weights_changes_the_next_dispatch() { /* weights [0] -> out A, weights [1] -> out B != A */ }
#[test] fn too_many_targets_fall_back_cleanly() { /* 129 целей -> GpuSkin::new отказывает без паники */ }
```

## Границы
- **Читать:** `skinning_gpu.rs` (целиком), `shaders/skin_compute.wgsl` (целиком), `skinning.rs` (render), `tests/gpu_skinning.rs`, `redox_anim/src/skinning.rs` (`morph_into`, `deform_morphed_into`), `mesh/compress.rs` (квантование).
- **Менять:** `skinning_gpu.rs`, `shaders/skin_compute.wgsl`, `tests/gpu_skinning.rs`; `renderer/memory.rs` (учёт буферов смещений в `MemoryCategory::Buffers`, **ПРОВЕРИТЬ** как учтены остальные буферы GPU-скиннинга).
- **Не трогать:** CPU-деформацию, `redox_anim`.

## Проверка
`cargo fmt`, `cargo clippy -p redox_render --all-targets -- -D warnings`, `cargo test -p redox_render --test gpu_skinning --test memory_wiring --test golden_frames`, `python scripts/check_file_sizes.py`.
Живая проверка владельцем: лицо с блендшейпами в `skinned_demo` (пример `crates/redox_core/examples/skinned_demo.rs`), ползунок веса.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Если WGSL-раскладка `Params` не сходится с `GpuSkinParams`, остановись и опиши фактические размеры.
