# FOL-03: отчёт исполнителя (шаги 1 и 5)

## Статус
- Шаг 1 (данные слоя и compute-отсечение без отрисовки): код написан, GPU-тесты написаны, **не запускались** (запрет координатора на cargo test). Проверка сборки: см. ниже.
- Шаг 5 (расстановка в `redox_scene`): код и тесты написаны, **не запускались**.
- Шаги 2, 3, 4, 6: не выполнялись (ждут MAT-08 и замер FOL-01).
- Ворота из спеки: FOL-03 стартует только после замера FOL-01 шаг 2. Выполнено по указанию координатора, не по замеру.

## Шаг 1: что сделано
Новые файлы:
- `crates/redox_render/src/foliage/mod.rs` - объявления модулей и реэкспорт.
- `crates/redox_render/src/foliage/layer.rs` - `LodRef`, `FoliageInstance`, `FoliageLayer`, `FoliageView`, `choose_lod`, `CullOutcome`, `cull_reference`.
- `crates/redox_render/src/foliage/cull.rs` - `FoliageGpu` (буферы слоя), `FoliageCull` (два compute-пайплайна и запись прохода).
- `crates/redox_render/shaders/foliage_cull.wgsl` - entry points `main` (отсечение) и `finalize` (indirect).
- `crates/redox_render/tests/foliage_gpu.rs` - три GPU-теста.

Правка: `crates/redox_render/src/lib.rs` - одна строка `pub mod foliage;` (с doc-комментом).

### Отклонения от спеки
- `cull_reference` принимает `(&FoliageLayer, &FoliageView, &[u32] previous)` и возвращает `CullOutcome { draws, lods }`, а не `(instances, camera, lods)`. Причина: гистерезис LOD требует состояния между кадрами, поэтому состояние передаётся явно и возвращается обратно.
- В `LodRef` добавлено поле `index_count`: без него `finalize` не может записать `DrawIndexedIndirect`.
- В `FoliageInstance` поле `_pad: [u32; 2]` ради 32-байтного шага WGSL-структуры (спека не упоминала).
- `FoliageLayer::bound_radius` добавлен: радиус сферы отсечения (`scale * bound_radius`) нужен шейдеру.
- Гистерезис реализован в GPU-тесте `lod_follows_the_distance_with_hysteresis` через буфер `prev_lod`, который живёт между кадрами.

### Форматы буферов (проверено по коду)
- `FoliageInstance` (Rust `repr(C)`, WGSL `FoliageInstance`): `pos: vec3<f32>` (0..12), `yaw` (12), `scale` (16), `seed: u32` (20), `_pad: vec2<u32>` (24..32). Шаг массива 32 байта. Совпадает с `array<FoliageInstance>`.
- `CullParams` (uniform, 160 байт, кратно 16): `planes: array<vec4<f32>,6>` (0..96), `camera_pos` (96), `lod_max_distance: vec4` (112), `lod_index_count: vec4<u32>` (128), `lod_count`, `instance_count`, `capacity` (144, 148, 152), `bound_radius` (156). Layout Rust `CullParams` повторяет его по полям.
- `prev_lod`: `array<u32>`, по одному на инстанс, стартовое значение `NO_LOD = 0xFFFFFFFF`, хранится между кадрами.
- `lod_counts`: `array<atomic<u32>, 4>`, очищается `encoder.clear_buffer` перед каждым кадром.
- `visible`: `array<u32>`, размер `MAX_LODS * capacity`; LOD `l` пишет в `[l*capacity, l*capacity + count_l)`. Переполнение отбрасывается (`slot < capacity`), `capacity = max(N, 1)`, поэтому не случается.
- `indirect`: `MAX_LODS * 5` u32 (20 байт на LOD): `index_count, instance_count, first_index=0, base_vertex=0, first_instance=0`. Пишет `finalize`.
- Буферы не нулевого размера, даже для пустого слоя.

### Логика
- `main`: frustum-тест сферы (`scale*bound_radius`) по 6 плоскостям из `virtual_geometry::Frustum::from_view_proj`; при отказе `prev_lod` не меняется. Далее `pick_lod`: сохранить предыдущий LOD внутри полосы `LOD_HYSTERESIS = 0.05`, иначе первый LOD с `d <= max_distance`, иначе `NO_LOD`.
- `finalize`: одна рабочая группа, пишет indirect-записи.
- CPU-эталон `cull_reference` повторяет ту же логику (`choose_lod`, те же плоскости).

### Команды для координатора
- Сборка: `cargo check -p redox_render -p redox_scene --all-targets`
- Тест отсечения: `cargo test -p redox_render --test foliage_gpu` (три теста: `the_gpu_cull_matches_the_cpu_reference`, `instances_behind_the_camera_are_culled`, `lod_follows_the_distance_with_hysteresis`)
- Тест расстановки: `cargo test -p redox_scene scatter_is_deterministic`, `cargo test -p redox_scene scatter_respects_slope_and_height`, `cargo test -p redox_scene adjacent_cells_do_not_overlap_instances`

### Риски шага 1
- WGSL: не проверено компилятором (naga). Возможные места: `array<atomic<u32>, MAX_LODS>` с override-константой, `if`-цепочки в `lod_far`. Первый же запуск `foliage_gpu` покажет.
- Сравнение на границе: CPU и GPU считают `sqrt` и плоскости в разном порядке сложения. Рассинхрон возможен только при точном совпадении расстояния с порогом; случайные 5000 инстансов его обычно не задевают.
- `write_buffer` для params и prev_lod: если в одном encoder будут несколько кадров, параметры перезапишутся. Для шага 2 нужен один dispatch на кадр или отдельный uniform на кадр.
- Вершинный шейдер шага 2 должен читать `visible[l*capacity + instance_index]`, а не `instance_index` напрямую.

## Шаг 5: что сделано
- `crates/redox_scene/src/pcg.rs`: `FoliageRule { model, density_per_m2, min_scale, max_scale, max_slope, height_range, mask: Option<fn(f32,f32)->bool> }`, `ScatteredInstance { pos, yaw, scale, seed }`, `scatter_layer(rule, cell, cell_size, seed, height_fn)`.
- Детерминизм: поток от `cell_seed(seed ^ fnv(model), cell)`; на каждого кандидата расходуется пять значений независимо от принятия, поэтому отказ одного места не сдвигает остальные.
- Инстансы лежат внутри квадрата ячейки `[cx*size, (cx+1)*size)`, соседние ячейки не пересекаются.
- Тесты в `#[cfg(test)] mod tests` в `pcg.rs`: `scatter_is_deterministic`, `scatter_respects_slope_and_height`, `adjacent_cells_do_not_overlap_instances`.

### Отклонения от спеки (шаг 5)
- `scatter_layer` принимает `cell_size` (без него нельзя знать границы ячейки). Возвращает `ScatteredInstance`, а не `FoliageInstance`: `redox_scene` не зависит от `redox_render` (и наоборот), поэтому конвертация поле за полем делается в вызывающем коде.
- Хранение в OFPA: проверено, `ofpa.rs` хранит только `ActorRecord` (kind, position, yaw, tags) в RON по одному файлу на актор. Для 40 000 пучков на ячейку это неприемлемо. Решение: экземпляры слоя **не** сохраняются, а пересчитываются из `(seed, rule, cell)`. Ограничение записано здесь; отдельный бинарный блок ячейки не сделан (вне шага 5).

## Точки интеграции для шага 2 (проверено чтением)
- `crates/redox_render/src/frame_graph.rs`: функция `frame_graph(width, height) -> RenderGraph` (строка 25) объявляет проходы и их чтение/запись. Комментарии о зависимостях на строках 64, 74, 120. Проход отрисовки слоя должен быть добавлен сюда и в `frame_pass_order()` (строка 154). Тест `every_declared_pass_has_an_implementation_in_the_renderer` (строка 181) проверит, что объявленный проход реализован в рендерере.
- `frame_graph_report` (строка 143) - текстовый отчёт графа, нужно обновить ожидания тестов, если они сравнивают текст.
- Проходы: `crates/redox_render/src/pass/` (новые проходы Forward/Velocity/Normal). Точка вызова записи в `renderer/frame.rs` - её не читал детально; проверить при шаге 2. `renderer/frame.rs` и `renderer/init.rs` сейчас правит FOL-01 - конфликт вероятен.
- Для шага 2 понадобятся: доступ к `FoliageGpu::visible_buffer()` и `indirect_buffer()` из прохода, вершинный шейдер с чтением `visible`, `Material` из `material_slot` и двусторонний режим от MAT-08.

## Что координатору проверить
1. Результат `cargo check -p redox_render -p redox_scene --all-targets` (см. раздел «Проверка» ниже).
2. Запуск трёх GPU-тестов `foliage_gpu` и трёх тестов `pcg`, первый запуск может выявить ошибки WGSL.
3. Ворота: FOL-03 начат без замера FOL-01.

## Проверка
- Компиляция НЕ подтверждена. Один `cargo check -p redox_render -p redox_scene --all-targets` был запущен в фоне и остановлен до конца; до остановки он выдал только два предупреждения `redox_render` (неиспользуемый импорт `FoliageInstance` в `cull.rs` и неиспользуемое поле в тесте), оба исправлены. Ошибок компиляции в выводе не было; `redox_scene` в выводе не дошёл.
- Затем по указанию владельца cargo больше не запускался ни в каком виде.
- Тесты (`foliage_gpu`, `pcg`) не запускались.
- Координатору прогнать по порядку: `cargo check -p redox_render -p redox_scene --all-targets`, `cargo test -p redox_scene scatter_`, `cargo test -p redox_render --test foliage_gpu`. Первый запуск может выявить ошибки WGSL (naga) и типов.
- Шаги 2, 3, 4, 6 не выполнялись.
