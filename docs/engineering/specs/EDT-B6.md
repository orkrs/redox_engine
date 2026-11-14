# EDT-B6 — Viewport редактора показывает VG, GI, IBL и режимы затенения

- **Статус:** не начат (аудит кода 2026-11-15).
- **Размер:** 4 шага (M, M, M, S) ≈ 8 дней. **Крейты:** `redox_editor`, `redox_editor_viewport`, `redox_render`. **Зависит от:** GI-07 (посев SDF), VG-06 (мешлеты в `VGSystem`), RFL-02 если в сцене есть пробы. **Блокирует:** —.
- **Модель:** haiku. Шаги 1–3 — с проверкой координатора на GPU.
- Закрывает вывод 4 в `docs/engineering/EDITOR_PLAN.md`: viewport рисует прокси-кубы и не показывает VG, GI и SDF. **Не входит:** новый рендерер редактора, импосторы, лайтмапы (LGT-01).

## Суть
`RenderHost` уже гоняет полный `Renderer` (PBR, VSM, GTAO, bloom, Sharp AA) в текстуру egui. Комментарий в `crates/redox_editor/src/render_host.rs` фиксирует дыры: нет сущностей `VirtualMesh`, GI/SDF не сеются, IBL — заглушка, небо выключено. Режимы Unlit, Normals, Roughness и Depth в `redox_editor_viewport/src/shading.rs` остаются `Unavailable` и рисуют Lit. Wireframe уже честный: линии поверх Lit, его не переписывать.

## Что уже есть
- `apply_viewport_runtime` выставляет `vg_use_vis_path` из `viewport.vg_enabled`. В настройках по умолчанию `vg_enabled: false` (`redox_editor_core/src/viewport_prefs.rs`).
- `ShaderDebugVizMode` умеет кластеры и отладку теней/GI, не умеет нормаль мира, roughness и глубину.
- Куб-прокси остаётся запасным путём, когда у актора нет меша.

## Шаг 1 — IBL и небо из проекта (M)
- Если в проекте есть HDR неба (тот же файл, что ест `bake-ibl`), viewport вызывает существующую загрузку IBL, а не dummy-окружение. Файла нет — оставить текущую заглушку и написать это в оверлее одной строкой.
- Небо включается вместе с успешным IBL.
Тест GPU: `viewport_with_an_hdr_is_not_the_dummy_environment` (средний цвет верхних строк кадра ближе к HDR, чем к заглушке; допуск зафиксировать по первому зелёному прогону и записать в отчёт).

## Шаг 2 — меши сцены вместо вечного прокси (M)
- Актор с путём меша даёт `RenderObject` из `ViewportMeshCache`. Прокси-куб — только если файла нет (это уже обещано комментарием; довести до теста).
- `vg_enabled: true` в настройках viewport по умолчанию, когда у адаптера есть нужные фичи. Нет фич — флаг остаётся false, оверлей объясняет почему.
- Инстансы VG появляются из тех же мешей (регистрация — VG-06). Пока VG-06 не влит, шаг включает флаг и рисует обычный меш; VG-путь не имитировать кубами.
Тесты: `a_missing_mesh_stays_a_proxy`; `a_present_gltf_draws_its_own_bounds` (AABB пикселей не равен кубу 1×1×1); `vg_defaults_off_when_the_adapter_lacks_features`.

## Шаг 3 — режимы затенения (M)
- Добавить в отладочный вид движка значения, которых не хватает `ViewportShadingPlan`: мир-нормаль, roughness, глубина, unlit (свет = 1, без IBL и теней).
- `resolve` для этих четырёх режимов возвращает `Available` и соответствующий `debug_mode`. Lit и Wireframe не меняются.
- Кадр режима отличается от Lit на сцене «серый шар + направленный свет».
Тесты GPU: `normals_mode_is_not_lit`; `roughness_mode_splits_a_smooth_and_a_rough_sphere`; `depth_mode_changes_with_distance`; `unlit_ignores_the_directional_light`. Юнит: `unavailable_reasons_are_gone_for_the_four_modes`.

## Шаг 4 — GI тумблер (S)
- Тумблер GI вызывает посев из GI-07. Тест из GI-07 считается приёмкой и здесь не дублируется, если уже зелёный. Если GI-07 ещё не влит — шаг помечается 📌 и не имитирует GI подменой albedo.

## Границы
- **Читать:** `EDITOR_PLAN.md` §1, `render_host.rs`, `shading.rs`, `viewport_runtime.rs`, EDT-B3.
- **Менять:** `render_host.rs`, `shading.rs`, шейдер отладочного вида, дефолт `vg_enabled`.
- **Не трогать:** порядок пост-цепочки, Packager.

## Проверка
`cargo fmt`, `cargo clippy -p redox_editor -p redox_editor_viewport -p redox_render --all-targets -- -D warnings`, юнит-тесты shading, GPU-тесты у координатора.

## Для исполнителя
`git merge --ff-only main`. Не подменять отсутствующий режим перекраской куба. Если режим нельзя выразить без второго цветового таргета — написать это в отчёт и оставить `Unavailable` с новой причиной, не с ложным `Available`.
