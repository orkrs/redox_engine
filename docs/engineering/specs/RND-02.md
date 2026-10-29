# RND-02 — Тангенты в GPU-скиннинге (compute-шейдер)

- **Статус:** выполнено 2026-10-29, принято в `main`. Знак ручности в шейдере приведён к правилу CPU (`compress_one`), добавлен тест на совпадение.

- **Размер:** M. **Крейт:** `redox_render`. **Зависит от:** ANM-01, RND-01. **Блокирует:** ничего.
- **Модель:** sonnet (шейдер + раскладка памяти + сверка с эталоном).

## Суть
`skin_compute.wgsl` пишет в вершину фиксированный тангент (`TANGENT_WORD = 0x007F0000u`, октаэдр от (1,0,0)).
CPU-путь после RND-01 даёт настоящие тангенты, GPU-путь — нет: картинка расходится.

## Границы
- **Читать:** `crates/redox_render/shaders/skin_compute.wgsl` (целиком, 120 строк), `src/skinning_gpu.rs` (`GpuSkinVertex` — 64 байта, `GpuSkin::new`, `bind_vertices`-заполнение, `run_batch`),
  `src/mesh/compress.rs` (`oct`-кодирование тангента, `CompressedVertex`), тесты `tests/gpu_skinning.rs`.
- **Менять:** `skin_compute.wgsl`, `skinning_gpu.rs`, `tests/gpu_skinning.rs`.
- **Не трогать:** остальные шейдеры, `skinning.rs` (кроме одной строки в комментарии, если нужно).

## Решение
Раскладка вершины (WGSL `SkinVertex` и Rust `GpuSkinVertex`) растёт с 64 до 80 байт: тангент идёт **после** весов (смещение 64, выравнивание vec3 = 16 — подходит):

```wgsl
struct SkinVertex {
    position: vec3<f32>, uv_packed: u32,      // 0..16
    normal: vec3<f32>,   _pad: u32,           // 16..32
    joints: vec4<u32>,                        // 32..48
    weights: vec4<f32>,                       // 48..64
    tangent: vec3<f32>,  has_tangent: u32,    // 64..80
}
```
```rust
#[repr(C)] struct GpuSkinVertex { /* ...as before... */ tangent: [f32; 3], has_tangent: u32 }   // size_of == 80, add a const assert
```
Шейдер: если `has_tangent != 0u`: `t = normalize((m * vec4(v.tangent, 0.0)).xyz)` (нулевая длина → `vec3(0.0)`), `ot = oct_encode(t)`,
слово `w2 = snorm8(o.x) | (snorm8(o.y) << 8u) | (snorm8(ot.x) << 16u) | (snorm8(ot.y) << 24u)`; иначе прежний `TANGENT_WORD`
(геометрия без тангентов ведёт себя как раньше). Кодирование тангента должно совпадать с CPU-кодированием в `compress.rs`
(тот же `oct_encode`, округление «от нуля» `round_half_away`; **знак/ручность** — как у нормали, как сейчас: `handed` считается по `n.z` — не менять).
Rust: `GpuSkin::new` заполняет `tangent` из `geometry.tangents` (или нули и `has_tangent = 0`).

## Пошагово
1. Тест сверки (ниже) сначала красный (тангенты расходятся).
2. Расширить структуры (Rust и WGSL одновременно: размер проверить const-assert'ом и тестом на старом `gpu_skinning`).
3. Шейдер → зелёный. Прогнать весь `gpu_skinning` (старые тесты: допуск до 3 шагов квантования не должен измениться).

## Тест (в `tests/gpu_skinning.rs`; пропуск без адаптера, как соседние)
```rust
#[test]
fn gpu_tangents_match_the_cpu_within_quantisation() {
    // geometry: demo_rig / chain_model with tangents = per vertex a unit vector perpendicular to the normal
    // (e.g. normal x Y, or X when parallel), pose: a bending chain (several bones, non-trivial rotation).
    // CPU: binding.update(...) -> read_mesh_vertex_bytes; GPU: a second binding, update_gpu -> bytes.
    // Compare tangent_oct of every vertex: max abs difference per i8 component <= 3.
}
#[test]
fn geometry_without_tangents_keeps_the_fixed_tangent_on_the_gpu() {
    // as before the change: tangent_oct == [127, 0] for every vertex.
}
```

## Проверка
`cargo test -p redox_render --test gpu_skinning`, `cargo run -p redox_core --example skinned_demo --release -- --golden` (кадры в допуске; если примеру нужен
файл эталона, он уже в репозитории), clippy.

## Для исполнителя
`git merge --ff-only main`. Не меняй формат `CompressedVertex`. Если в `compress.rs` кодирование тангента расходится со словом в шейдере (порядок байт), сообщи, а не подгоняй допуск.
