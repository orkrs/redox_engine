# RND-06 — Разбить `renderer/frame.rs` (826 строк) по проходам

- **Статус:** не начат.

- **Размер:** S (3 шага по XS–S) ≈ 1 день. **Крейт:** `redox_render`. **Зависит от:** — (выполнять **до** RND-09 и GI-*, чтобы они не росли в `frame.rs`). **Блокирует:** —.
- **Модель:** haiku.
- Чистый рефакторинг: **поведение и кадры не меняются**; мягкий лимит файла 800 строк превышен (826).

## Суть
`renderer/frame.rs` содержит `render_frame_into` (диспетчер проходов по `frame_pass_order()`, строки 61–253) и методы `pass_*` (строки 254–790): `pass_ddgi_fill` (254), `pass_cluster_vg_cull` (276), `pass_shadows` (324), `pass_normal_prepass_hiz` (413), `pass_forward` (472), `pass_vg_visibility_resolve` (523, 126 строк), `pass_transparent` (649), `pass_msaa_resolve_particles` (686), `pass_frame_tail` (724). Уже вынесены `frame/post.rs` и `frame/shadows.rs`.

## Шаги
1. **VG-проход:** `pass_cluster_vg_cull` и `pass_vg_visibility_resolve` → новый файл `renderer/frame/vg.rs` (`impl Renderer { pub(super) fn … }`; объявление `mod vg;` в `frame.rs`).
2. **Основная геометрия:** `pass_normal_prepass_hiz`, `pass_forward`, `pass_transparent`, `pass_msaa_resolve_particles` → `renderer/frame/geometry.rs`.
3. **GI и хвост:** `pass_ddgi_fill` (в будущем GI-проходы) → `renderer/frame/gi.rs`; `pass_frame_tail` оставить в `frame.rs` вместе с диспетчером.
Ожидаемо `frame.rs` ≈ 250–300 строк. Видимость: методы `pub(super)`/`pub(crate)` как были; никаких изменений в сигнатурах, логике, порядке, комментариях (перенос текстом). `use super::*;` в новых файлах, как в `frame/post.rs`.

## Тесты и приёмка
Новых тестов нет — проверка «ничего не изменилось»:
- `cargo test -p redox_render` (весь крейт) зелёный без правок тестов; `tests/golden_frames.rs` побитно; тест `frame_graph::every_declared_pass_has_an_implementation_in_the_renderer` (читает `include_str!("renderer/frame.rs")`!) — **обновить** тест так, чтобы он собирал текст из `frame.rs` + `frame/*.rs` (иначе он упадёт, потому что сверяет имена методов по исходнику `frame.rs`); правка теста — единственное допустимое изменение вне переноса. **ПРОВЕРИТЬ** `frame_graph.rs:~117` (`FRAME_SOURCE`).
- `python scripts/check_file_sizes.py` без предупреждения по `frame.rs`.
- `git diff --stat` показывает только перенос: сумма добавленных строк ≈ удалённых (± `mod`/`use`).

## Границы
- **Читать:** `renderer/frame.rs`, `renderer/frame/{post,shadows}.rs`, `frame_graph.rs`, `renderer/mod.rs`.
- **Создать:** `renderer/frame/{vg,geometry,gi}.rs`.
- **Менять:** `renderer/frame.rs`, `frame_graph.rs` (только тест).
- **Не трогать:** остальные файлы.

## Проверка
`cargo fmt`, `cargo clippy -p redox_render --all-targets -- -D warnings`, `cargo test -p redox_render`, `cargo check --workspace --all-targets`, `python scripts/check_file_sizes.py`, `python scripts/gen_crates_doc.py --check`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Каждый шаг — отдельный коммит; после каждого `cargo test -p redox_render --test golden_frames`. Не «улучшай» код при переносе.
