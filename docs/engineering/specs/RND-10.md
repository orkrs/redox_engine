# RND-10 — Проход скорости движущихся объектов и прозрачные: измерить, затем слить (по воротам)

- **Статус:** не начат. **Ворота:** шаги 2–3 только если шаг 1 покажет выигрыш ≥ 0,3 мс GPU или ≥ 15 % вызовов отрисовки на «тяжёлой» сцене.

- **Размер:** 3 шага (S, M, S) ≈ 4–5 дней. **Крейт:** `redox_render`. **Зависит от:** RND-04 (замер кадра, готов), PST-01 (скорость, готов), RND-09 (желательно: проходы объявлены в графе). **Блокирует:** —.
- **Модель:** haiku по шагам 1, 3; шаг 2 (шейдеры, MRT) — haiku с проверкой координатора.
- Закрывает пункт «прозрачные объекты и проход скоростей движущихся объектов не объединены так, как хотелось бы для оптимизации». **Не входит:** GPU-driven сцена (RND-07 закрыт по замеру, 1,4–1,6 мс CPU), переписывание порядка прозрачных.

## Суть и аудит
- Объекты группируются по `(меш, материал)` в CPU-инстансинг (`renderer/batching.rs`), прозрачные рисуются отдельным проходом по убыванию расстояния (`pass/transparent.rs`, `renderer/draw.rs:107`), аддитивные — после (`draw.rs:175`).
- Скорость камеры: полноэкранный проход по глубине (`pass/velocity.rs`, `velocity.wgsl`); скорость **движущихся** объектов: отдельный проход `object_velocity.wgsl`, который заново рисует их меши (`renderer/motion.rs:88 record_object_velocity`) — то есть движущиеся объекты рисуются дважды (основной + скорость), плюс прозрачные в скорость не попадают вообще.
- Решение «оптимально»: **если** замер подтвердит стоимость — писать скорость объекта вторым выводом (MRT) основного прохода непрозрачных (`pbr.wgsl` → `@location(1) velocity`), а отдельный проход оставить как запасной (флаг). Прозрачные: **не** писать скорость (смешение прозрачных в скорость даёт ложные векторы); вместо этого — реактивная маска TAA (MAT-06 шаг 5).

## Шаг 1 — замер (S)
`tests/velocity_cost.rs` (`#[ignore]`, release, печать): сцена «20 движущихся персонажей-мешей по 5k вершин + 3000 статических кубов + 50 прозрачных»; режимы: (а) как сейчас; (б) без прохода скорости объектов (флаг `Renderer::debug_skip_object_velocity(true)` — тестовая функция, `#[doc(hidden)]`). Метрики: GPU-время кадра (`profiler.rs`, метки проходов), число вызовов отрисовки (`forward_draw_calls()`), CPU-запись (`last_frame_cpu_timings`). Результат — таблица в `docs/BENCHMARKS.md`, вывод **«сливать / не сливать»** и подпись координатора в «Статус».
Тесты: `the_debug_flag_removes_exactly_the_object_velocity_pass` (счётчик проходов), `skipping_the_pass_leaves_camera_motion_intact` (скорость неподвижного объекта та же).

## Шаг 2 — скорость вторым выводом основного прохода (M, если «сливать»)
- `pbr.wgsl`: второй цветовой вывод `velocity: vec2` = `prev_clip.xy/w − curr_clip.xy/w` (в UV, как `object_velocity.wgsl`; зеркало формулы), для статичных объектов `prev_model == model` → 0 → объединяется с камерной скоростью по правилу `object_velocity.wgsl` (**ПРОВЕРИТЬ** как складываются: объект перезаписывает скорость; камерная часть добавляется из `velocity.wgsl`, который сейчас читает глубину; после слияния — камерная скорость считается во втором полноэкранном проходе по пикселям, где объектная скорость не записана: маска `w = 0`).
- Предыдущая модельная матрица на объект: уже есть (`motion.rs`, `update_object_motion`, буфер предыдущих матриц) — добавить в `ModelUniform`/хранилище, если ещё не там (изменение раскладки — с тестом размера).
- Цель `velocity` — существующая `taa_velocity_tex`; формат `Rg16Float`/`Rgba16Float` (как сейчас); MSAA-ветка: velocity не мультисэмплится (как сейчас) — **ПРОВЕРИТЬ** путь MSAA и либо оставить старый проход там, либо писать во второй цели 1×.
- Флаг `Renderer::set_merged_velocity(bool)` (по умолчанию true только если шаг 1 дал «сливать»); `false` — прежний путь побитно.
Тесты (GPU): `merged_velocity_equals_the_separate_pass` (на 5 кадров с движущимся кубом — различие скорости ≤ 1e-3 по пикселям куба), `static_objects_get_only_camera_motion` , `the_merged_path_removes_the_second_draw_of_moving_objects` (счётчик вызовов отрисовки уменьшился ровно на число групп движущихся), `taa_ghost_metrics_do_not_regress` (метрики PST-04 в тех же допусках), `with_the_flag_off_the_frame_is_bitwise_unchanged` (golden).

## Шаг 3 — прозрачные: единый список и порядок (S)
Без изменения картинки: `draw_list` для прозрачных строится одним проходом по `RenderObject` (сейчас — `has_alpha_objects` + отдельный обход), сортировка по расстоянию кэшируется между кадрами, если камера и объекты не двигались (флаг `unchanged_since_last_frame`); аддитивные идут своим списком без сортировки. Измерить CPU (`last_frame_cpu_timings`) до/после на 500 прозрачных объектах; оставить изменение, только если CPU ≥ −15 %.
Тесты: `transparent_order_is_back_to_front` (регресс `transparency`), `a_static_scene_reuses_the_sort` (счётчик сортировок), `moving_the_camera_resorts`, `additive_objects_are_not_sorted`.
Документы: `PLAN_EXECUTION.md` A2 (результат), `CHANGELOG.md`, `docs/BENCHMARKS.md`.

## Границы
- **Читать:** `renderer/{motion,draw,batching,frame}.rs`, `pass/{velocity,pbr,transparent}.rs`, `shaders/{velocity,object_velocity,pbr}.wgsl`, `profiler.rs`, `tests/object_velocity.rs`, `tests/wave7_gpu.rs`.
- **Создать:** `tests/velocity_cost.rs`.
- **Менять (шаги 2–3):** `shaders/pbr.wgsl`, `shaders/velocity.wgsl`, `pass/{pbr,velocity}.rs`, `renderer/{motion,draw,frame,attachments}.rs`.
- **Не трогать:** VG-пути (`vg_resolve` пишет свою скорость отдельно), тени.

## Проверка
`cargo fmt`, `cargo clippy -p redox_render --all-targets -- -D warnings`, `cargo test -p redox_render --test object_velocity --test transparency --test post_fx --test golden_frames --test instancing`, `cargo check --workspace --all-targets`, `python scripts/check_file_sizes.py`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Выполни **только шаг 1** и отчитайся таблицей. Шаги 2–3 — после записи «сливать» в «Статус». Любая смена GPU-структуры — вместе с WGSL и тестом размера.
