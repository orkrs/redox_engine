# RND-01 — Тангенты в CPU-скиннинге (`SkinnedMeshBinding`)

- **Статус:** выполнено 2026-10-29, принято в `main`.

- **Размер:** S. **Крейт:** `redox_render`. **Зависит от:** ANM-01. **Блокирует:** RND-02.
- **Модель:** haiku.

## Суть
`bind_vertices` (`crates/redox_render/src/skinning.rs`, около строки 213) ставит каждой вершине `DEFAULT_TANGENT = [1,0,0]`,
`SkinnedMeshBinding::update` (строка ~145) обновляет только позиции и нормали. После ANM-01 геометрия может нести тангенты.

## Границы
- **Читать:** `skinning.rs` (структура `SkinnedMeshBinding`, `new`, `update`, `bind_vertices`, тест `bind_vertices_copy_geometry`),
  `redox_anim::SkinnedGeometry::{has_tangents, deform_tangents_into}` (ANM-01), `mesh/mod.rs` (`Vertex.tangent: [f32;3]`).
- **Менять:** только `crates/redox_render/src/skinning.rs`.
- **Не трогать:** `skinning_gpu.rs`, шейдеры (это RND-02), `redox_anim`.

## Решение
1. Поле `tangents: Vec<[f32; 3]>` в `SkinnedMeshBinding` (буфер для повторного использования, как `positions`/`normals`).
2. `bind_vertices`: `tangent = geometry.tangents.get(i).copied().unwrap_or(DEFAULT_TANGENT)`.
3. `update`: после `deform_into` вызвать `geometry.deform_tangents_into(skin_matrices, &mut self.tangents)`; в цикле присвоения
   `vertex.tangent = tangent` **только если** `!self.tangents.is_empty()` (иначе оставить значение из bind-позы).
4. `DEFAULT_TANGENT` и его комментарий («normal maps are not skinned yet») обновить: тангент по умолчанию нужен только геометрии без тангентов.
5. `update_gpu`/`update_gpu_batch` не менять (до RND-02 GPU-путь по-прежнему использует фиксированный тангент; зафиксировать это
   в doc-комментарии `update_gpu`).

## Пошагово
1. Тесты (ниже) → красные.
2. Реализация → зелёные.

## Тесты (модуль `tests` в `skinning.rs`, рядом с `bind_vertices_copy_geometry`; без GPU)
```rust
#[test]
fn bind_vertices_use_geometry_tangents_when_present() {
    let mut g = redox_anim::procedural::chain_model(2, 0.5, 0.1, 6).geometry;
    g.tangents = (0..g.vertex_count()).map(|i| [0.0, 0.0, 1.0 + i as f32]).collect();
    let v = bind_vertices(&g);
    assert_eq!(v[3].tangent, [0.0, 0.0, 4.0]);
}
#[test]
fn bind_vertices_fall_back_to_the_default_tangent() {
    let g = redox_anim::procedural::chain_model(2, 0.5, 0.1, 6).geometry;
    assert!(!g.has_tangents());
    assert!(bind_vertices(&g).iter().all(|v| v.tangent == DEFAULT_TANGENT));
}
```
И GPU-тест в `tests/gpu_skinning.rs` (пропускается без адаптера, как соседние): геометрия `chain_model` с тангентами `[1,0,0]`, поза с поворотом на 90° вокруг Z одной кости;
после `binding.update(&renderer, &skin)` вершины, привязанные только к этой кости, в буфере меша имеют тангент ≈ `[0,1,0]` (читать через
`renderer.read_mesh_vertex_bytes(slot)`, формат — `CompressedVertex`, декодировать тангент октаэдрически; готовая функция декодирования — искать в `mesh/compress.rs`
(`decode`), если её нет — сравнить с кодированием ожидаемого вектора через `CompressedVertex::from_vertex` с допуском 2 шага).

## Проверка
`cargo test -p redox_render skinning`, `cargo test -p redox_render --test gpu_skinning`, clippy `-p redox_render`.

## Для исполнителя
`git merge --ff-only main`; убедись, что ANM-01 уже в `main` (в `SkinnedGeometry` есть `tangents`) — иначе остановись.
