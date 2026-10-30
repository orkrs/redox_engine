# RND-09 — Граф кадра: ресурсно-ориентированный render graph (авто-зависимости, lifetime, реальное переиспользование памяти, проверки)

- **Статус:** частично. Сделано: шаг 1 (типы доступа, буферы, проверка флагов, `SelfDependency`, план барьеров, критический путь, тесты в `render_graph.rs`); шаг 2 (debug-сверка обращений): `FrameUsage` (`renderer/frame/usage.rs`), `FrameState::touch` в проходах и диспетчере, `Renderer::frame_usage_mismatches()`, тест `tests/frame_graph_runtime.rs` (`#[ignore]`: остались чтения через bind group и внутренние проходы, см. ниже). Объявления выровнены с кодом (`frame_graph.rs`): `hdr_resolved` удалён (msaa и scatter пишут `hdr_color`), `shadow_mask` заменён на `vsm_shadow_filtered`, добавлены импорты `material_surface`, `vg_vis_targets`, `taa_velocity`, `ldr`, `smaa_out`; промежуточные цели поста объявлены `read_write`. Не покрыто шагом 2: обращения через bind group и внутренние цели (`passes.rs`, `post.rs`, `vg_vis_targets`, `material_surface`) не записываются; `PassResources` как обёртка доступа не введён (см. отчёт). Шаг 3 не начат: сверка шага 2 не проходит (не записаны чтения `depth`/`shadow_maps`/`clusters`/`ao` через bind group, `hiz` в `cluster_vg_cull` (чтение предыдущего кадра), `vsm_pool` в `shadow_passes`, `backbuffer` в `frame_tail`), а тесты wgpu-аллокатора не запускались. Не сделано: шаги 3–5 (координатор запустит позже), шаг 6 без `Renderer::frame_graph_report` и без `to_dot`-раскраски слотов.

- **Размер:** 6 шагов (S, M, M, M, M, S) ≈ 10–12 дней. **Крейт:** `redox_render`. **Зависит от:** — (лучше после RND-06: разбиение `frame.rs`). **Блокирует:** — (но GI-01…04, PST-06 шаг 3 добавляют проходы и текстуры: после этой задачи их объявляют в графе).
- **Модель:** haiku по шагам 1, 2, 6; шаги 3–5 (аллокатор, миграция проходов) — haiku с проверкой координатора после каждого шага.
- Объём: «перевести граф с декларативной подсказки на источник истины». **Не входит:** многопоточная запись команд по проходам, async compute (wgpu не даёт очередей).

## Суть и аудит (по коду на 2026-10-29)
- `render_graph.rs` (553 строки, чистая CPU-логика): `create_texture` / `import_texture`, `add_pass(name, |p| p.read(t).write(t).side_effects())`, `compile()` → порядок, отсечение проходов, `lifetimes: Vec<Option<(first, last)>>`, слоты для транзиентных текстур с равными `TextureDesc`, `to_dot()`. Зависимости **выводятся** из `read/write` (последний писатель → читатель; WAR/WAW по порядку объявления), чтение транзиента до записи — ошибка.
- `frame_graph.rs` (170 строк): объявляет **13 крупных проходов** вручную; `frame_pass_order()` задаёт порядок, `renderer/frame.rs` выполняет по методу на проход. Тест сверяет, что для каждого объявленного прохода есть метод.
- Но: (1) то, что проход *на самом деле* читает и пишет, никак не проверяется — объявление и код расходятся молча; (2) текстуры **принадлежат проходам** (`ao`, `shadow_mask`, `hdr`, `hdr_resolved`, bloom, TAA и т. д. создаются в `renderer/init.rs`, `resize.rs`, `attachments.rs`), план слотов «информационный»; (3) буферы (списки кластеров, сетка света) в графе не существуют; (4) проходы слишком крупные (`taa_tonemap_bloom_smaa`) — поэтому выигрыша от переиспользования нет (архитектурный документ: «ни одна пара транзиентных текстур не даёт выигрыша»); (5) выключенный эффект не убирает свои ресурсы.
- Ограничение wgpu: **нет размещённых ресурсов (placed resources) и нет ручных барьеров** — барьеры вставляет сам wgpu. Поэтому «алиасинг» = **один физический `wgpu::Texture` на несколько логических текстур с непересекающимися lifetime и совместимыми описаниями**, а «переходы состояний» = проверка, что объявленное использование соответствует флагам `TextureUsages` и порядку, плюс отчёт.

## Шаг 1 — типы доступа, буферы и расширенная компиляция (S, чистая логика)
- `PassBuilder`: оставить `read`/`write`; добавить `read_as(tex, Access)`, `write_as(tex, Access)`, `Access::{ColorTarget, DepthTarget, ShaderRead, StorageRead, StorageWrite, CopySrc, CopyDst}` (`read`/`write` = `ShaderRead`/`ColorTarget` по умолчанию).
- Буферы: `BufferDesc { size: u64, usage: wgpu::BufferUsages }`, `create_buffer` / `import_buffer`, `BufHandle`, `read_buf` / `write_buf` (+ `_as`). Те же правила зависимостей и lifetime.
- `compile()` дополнительно: `required_usage(handle) -> TextureUsages` (объединение по `Access` всех проходов), `GraphError::MissingUsage { resource, needed, declared }` когда `TextureDesc.usage` не покрывает нужное; `hazards: Vec<Hazard>` (запись и чтение одного ресурса в одном проходе без `read_write`-пометки — ошибка `SelfDependency`); `critical_path_len`; `barrier_plan: Vec<Transition { resource, from: Access, to: Access, before_pass }>` (информационно, используется отчётом и тестами).
Тесты (в `render_graph.rs`/`tests/render_graph.rs`):
```rust
#[test] fn the_usage_flags_are_checked_against_the_declared_access() { /* StorageWrite без STORAGE_BINDING -> MissingUsage */ }
#[test] fn a_buffer_dependency_orders_the_passes() { /* кластеры: write_buf затем read_buf */ }
#[test] fn a_pass_that_reads_and_writes_the_same_texture_is_reported() { /* SelfDependency, если не через read_write */ }
#[test] fn the_barrier_plan_lists_every_change_of_access() { /* ColorTarget -> ShaderRead между двумя проходами */ }
#[test] fn an_unused_buffer_is_culled_with_its_producer() { /* ... */ }
#[test] fn the_critical_path_of_a_chain_equals_its_length() { /* ... */ }
```

## Шаг 2 — декларация становится истиной: проверка доступа во время работы (M)
- `PassResources<'a>` — выдаётся каждому проходу: `texture(handle) -> &TextureView`, `buffer(handle) -> &Buffer`. Попытка получить ресурс, **не объявленный** проходом, в debug-сборке паникует с именем прохода и ресурса, в release — `log::error!` один раз и возврат запасного ресурса (чёрная текстура) — **ПРОВЕРИТЬ** политику паник в проекте (`RND-05` убрал паники инициализации; в кадре допустим `debug_assert!`).
- Метод каждого прохода в `renderer/frame.rs` получает `&PassResources` (подпись `fn pass_x(&mut self, f: &mut FrameState<'_>, res: &PassResources)`): на этом шаге `PassResources` только **проверяет** доступы к ресурсам, которые проходы всё ещё берут из полей `Renderer`: каждое `self.hdr_view` и т. п. оборачивается в `res.texture(h)`, где `h` — handle из графа; поля остаются владельцами (миграция — шаги 3–4).
- Тест декларации: `every_declared_read_and_write_is_used_by_the_pass` — запуск кадра с счётчиком обращений (`res.usage_log()`), сверка: множество фактически затронутых = объявленному (лишние объявления и необъявленные обращения — ошибка теста). Требует GPU-кадра: тест в `tests/frame_graph_runtime.rs`, пропуск без адаптера.

## Шаг 3 — транзиентный аллокатор: физические текстуры по слотам (M)
- `TransientPool`: по `CompiledGraph` создаёт по одному `wgpu::Texture` на **слот** (описание = `TextureDesc` с `usage` = объединение `required_usage` всех логических текстур слота); `view(handle)` отдаёт вид слота; логические текстуры слота записаны в таблицу `slot_users`.
- Пересоздание: при `resize` (размер экрана) и при смене набора включённых эффектов (шаг 5); старые слоты, не изменившиеся по описанию, **переиспользуются** (не пересоздаются).
- Совместимость алиасинга (ужесточить в `render_graph.rs`): общий слот допустим, только если равны `width`, `height`, `format`, `sample_count` и `mip_level_count`; `usage` берётся объединением (раньше — требовалось равенство).
- Очистка: переиспользованная текстура содержит чужие данные — проход, читающий текстуру до первой записи, невозможен (`ReadBeforeWrite` уже ловит); проход, пишущий **часть** текстуры (например, маска), обязан объявить `write_partial` → компилятор запрещает алиасинг такой текстуры с другими (тест).
- Учёт памяти: `MemoryCategory::Transient` (**ПРОВЕРИТЬ** `memory_budget.rs`, `renderer/memory.rs`), в отчёт добавить «aliased_saved_bytes» = сумма размеров логических − сумма размеров слотов.
Тесты:
```rust
#[test] fn two_disjoint_textures_share_one_physical_texture() { /* GPU: два слота в один, проверить, что wgpu::Texture::global_id совпадает / pool.slot_count() == 1 */ }
#[test] fn overlapping_textures_never_share() { /* ... */ }
#[test] fn the_pool_rebuilds_only_changed_slots_on_resize() { /* счётчик создания */ }
#[test] fn a_partially_written_texture_is_never_aliased() { /* ... */ }
#[test] fn the_saved_bytes_are_reported() { /* ... */ }
```

## Шаг 4 — миграция проходов на ресурсы графа (M)
Переносить по одному, каждый — отдельный коммит с зелёными `golden_frames` и `post_fx` (кадры побитно прежние: изменение владельца памяти не меняет картинку):
1. `ao` и `shadow_mask` (R8Unorm, полный экран).
2. `hdr_color` / `hdr_resolved` (Rgba16Float).
3. `normals`, `surface` (roughness/metallic), `depth` (если не импорт из `attachments.rs`).
4. Цепочка пост-эффектов `PostFx` (SSR, motion blur, DoF, туман, SSS-размытие): временные ping-pong цели получают handle'ы графа.
5. Bloom: цепочка мипов (оставить как есть, если выигрыша нет — записать в отчёт, не мигрировать ради галочки).
Импортируемые (персистентные) ресурсы остаются владельцами: `vsm_pool`, `taa_history`, `ibl`, атласы GI.
**ПРОВЕРИТЬ** перед каждым пунктом, где создаётся текстура: `grep -rn "create_texture" crates/redox_render/src/renderer crates/redox_render/src/post | head -50` и составить в отчёте таблицу «текстура → владелец до → после».

## Шаг 5 — гранулярность и условные проходы: выигрыш должен быть измерен (M)
- Разбить крупные проходы в `frame_graph.rs` так, чтобы lifetimes стали короче: `taa_tonemap_bloom_smaa` → `taa`, `ssr`, `motion_blur`, `dof`, `fog`, `bloom_down`, `bloom_up`, `tonemap`, `smaa` (каждый — реальная функция в `frame.rs`; поведение не менять). `frame_pass_order()` и тест «у каждого прохода есть реализация» остаются.
- Условные проходы: `frame_graph(width, height, features: FrameFeatures)` — `FrameFeatures { ssao, ssr, fog, motion_blur, dof, vg, gi, … }` (битовая маска из настроек `Renderer`); выключенный эффект **не объявляется** → его ресурсы не создаются и проход не выполняется. Результат `compile()` кэшируется по `(w, h, features)` (`HashMap`); пересборка пула только при смене ключа.
- Замер: `tests/frame_graph_runtime.rs` (`#[ignore]`, печать) — для 1080p: байты логических транзиентных текстур, байты физических слотов, экономия, число слотов, до/после для режимов «всё включено» и «минимум». **Честная цель:** экономия > 0 при включённой цепочке пост-эффектов (ping-pong цели одного описания); если для базового режима выигрыша нет, записать число и не изобретать искусственные проходы.

## Шаг 6 — отчёт, отладка, документация (S)
- `Renderer::frame_graph_report() -> String`: проходы в порядке, у каждого — ресурсы (чтение/запись), lifetime, слот, барьерный план, отсечённые проходы; `to_dot()` с цветами слотов; кнопка F6 в примерах не нужна (достаточно метода).
- Документ: `docs/ARCHITECTURE_REALITY.md` (раздел графа: таблица «было/стало», честно про wgpu без placed resources), `PLAN_EXECUTION.md` (строка про граф кадра), `CHANGELOG.md`, `docs/BENCHMARKS.md` (цифры шага 5).

## Границы
- **Читать:** `render_graph.rs`, `frame_graph.rs`, `renderer/{frame,init,resize,attachments,memory,post}.rs`, `post/post_fx.rs`, `memory_budget.rs`, `tests/{post_fx,golden_frames,resize_bloom}.rs`.
- **Создать:** `src/transient_pool.rs`, `tests/{render_graph,frame_graph_runtime}.rs`.
- **Менять:** `render_graph.rs`, `frame_graph.rs`, `renderer/{frame,init,resize,attachments,mod,memory}.rs`, `post/*` (получение целей), `memory_budget.rs`, `lib.rs`.
- **Не трогать:** шейдеры, VG-рантайм, VSM-пулы (импорт).

## Проверка
`cargo fmt`, `cargo clippy -p redox_render --all-targets -- -D warnings`, `cargo test -p redox_render --test render_graph --test frame_graph_runtime --test golden_frames --test post_fx --test resize_bloom --test memory_report`, `cargo check --workspace --all-targets`, `python scripts/check_file_sizes.py` (`frame.rs` не должен расти: новые проходы выносить в `renderer/frame/*.rs`).

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Шаги по порядку, каждый — отдельный коммит. После шагов 3–5 кадры должны быть **побитно** прежними (тесты golden). Если какой-то проход нельзя перевести без смены картинки — остановись, опиши причину, остальные не трогай.
