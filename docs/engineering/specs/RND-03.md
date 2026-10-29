# RND-03 — Учёт категории `Buffers` в отчёте памяти

- **Статус:** выполнено 2026-10-29, принято в `main`. Учтены и `face_matrix_buffers`; двойного счёта `vg_material_buffer` нет (он не входит в `gpu_bytes`).

- **Размер:** XS. **Крейт:** `redox_render`. **Зависит от:** ничего. **Блокирует:** ничего.
- **Модель:** haiku.

## Суть
`Renderer::memory_usage()` (`crates/redox_render/src/renderer/memory.rs`) считает меши, текстуры, тени, VG и цели рендера. Категория
`MemoryCategory::Buffers` (`memory_budget.rs:72`) всегда пуста, хотя в `Renderer` есть постоянные буферы.

## Границы
- **Читать:** `renderer/memory.rs` (метод `memory_usage`), `renderer/mod.rs` (поля `wgpu::Buffer`, строки 148, 160, 162, 188, 275, 313, 377, 426, 430, 473),
  `tests/memory_wiring.rs` (образец).
- **Менять:** `renderer/memory.rs`, `tests/memory_wiring.rs`.
- **Не трогать:** `mod.rs`, `memory_budget.rs`.

## Решение
В `memory_usage` после целей рендера:
```rust
let buffers = [
    &self.camera_buffer, &self.light_buffer, &self.shader_debug_buffer, &self.tonemap_uniform_buffer,
    &self.model_buffer, &self.tonemap_rcas_uniform_buffer, &self.prev_model_buffer,
    &self.vsm_pageinfo_buffer, &self.vsm_info_buffer, &self.vg_material_buffer,
];
for buffer in buffers { budget.add(MemoryCategory::Buffers, buffer.size()); }
```
Если поле приватное в `mod.rs`, а `memory.rs` — вложенный модуль `renderer`, доступ есть (так уже читаются `hdr_texture` и др.). Если часть
буферов хранится в подсистемах (`cluster_manager`, `taa`, VG уже учтён отдельно) — не добавлять, а записать в комментарий «не учтено: …» с перечнем (поиск: `grep -rn "wgpu::Buffer" crates/redox_render/src --include=*.rs -l`).
Обновить doc-комментарий метода (строка про `Other`): добавить «Buffers — постоянные uniform/storage-буферы рендерера».

## Тесты (добавить в `tests/memory_wiring.rs`)
```rust
#[test]
fn a_fresh_renderer_accounts_its_persistent_buffers() {
    let Some(r) = renderer() else { return };
    let usage = r.memory_usage();
    assert!(usage.used(MemoryCategory::Buffers) >= 1024, "camera/light/model buffers");
    assert_eq!(usage.used(MemoryCategory::Buffers), r.memory_usage().used(MemoryCategory::Buffers), "stable");
}
#[test]
fn the_report_names_the_buffers_line() {
    let Some(r) = renderer() else { return };
    assert!(r.memory_report().contains("buffers:"));
}
```
(Формат строки отчёта сверить с `MemoryBudget::report()` в `memory_budget.rs`; если название категории выводится иначе, поправить ожидание теста.)

## Проверка
`cargo test -p redox_render --test memory_wiring --test memory_report`, clippy.

## Для исполнителя
`git merge --ff-only main`. Готово, когда оба теста зелёные и F5-оверлей (без правок) показывает строку buffers.
