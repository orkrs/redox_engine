# ANM-04 — glTF-скелет и клипы в графе анимации (Courier), порог GPU/CPU скиннинга

- **Статус:** не начат.

- **Размер:** 4 шага (S, M, S, S) ≈ 6 дней. **Крейты:** `redox_anim`, `redox_render`, `redox_core` (пример). **Зависит от:** ANM-02/03 (готовы), RND-01/02 (готовы). **Блокирует:** —.
- **Модель:** haiku по шагам 1, 3, 4; шаг 2 — haiku с проверкой координатора.
- Закрывает два пункта: (а) «скелет и клипы из glTF не полностью подключены к графу анимации в рабочем примере Courier»; (б) «GPU-скиннинг по одному персонажу медленнее CPU на малых мешах — нужен порог». Нормали и тангенты в скиннинге уже сделаны (ANM-01, RND-01, RND-02).

## Суть и аудит
- `redox_anim::gltf_import::load_glb(bytes) -> Vec<SkinnedModel>` читает скелет, геометрию, клипы (`AnimationClip` по каналам костей + веса морфов). `AnimGraph`/`GraphPlayer` (`graph.rs`) играют клипы **по именам** (state machine, blend tree, слои с маской, root motion, события) на `Skeleton`.
- Foxfire (`examples/foxrun/fox.rs:58`) берёт лису из glTF; **Courier** строит собственный `rig::graph()` на процедурных клипах (`examples/courier/game.rs:129`) — не использует glTF-скелет и клипы.
- Пакетный GPU-скиннинг есть (100 персонажей 0,31 мс в release, A3); порог «одиночный персонаж: CPU или GPU» не измерен, режим выбирается вручную (**ПРОВЕРИТЬ** `SkinnedMeshBinding`, `skinning_gpu.rs`, какой путь по умолчанию).

## Шаг 1 — привязка glTF к графу: имена клипов, проверка и сопоставление костей (S, `redox_anim`)
- `redox_anim/src/graph_binding.rs`:
```rust
/// What a graph needs from a model: the clip names its states play and the bone names its layer masks reference.
pub fn required_clips(graph: &AnimGraph) -> Vec<String>;
pub fn required_bones(graph: &AnimGraph) -> Vec<String>;
/// Checks the model against the graph; returns every missing clip/bone, not only the first.
pub fn validate_binding(graph: &AnimGraph, model: &SkinnedModel) -> Result<(), Vec<BindingError>>;
/// Maps a graph's clip names to model clip names (case-insensitive, trims `armature|` prefixes), explicit overrides win.
pub fn resolve_clip_names(graph: &AnimGraph, model: &SkinnedModel, overrides: &[(&str, &str)]) -> Result<ClipTable, Vec<BindingError>>;
```
- `ClipTable` — `HashMap<String, usize>` индексов клипов модели, который принимает `GraphPlayer::update` (**ПРОВЕРИТЬ** тип `clips` в `update`, строка `game.rs:181`).
Тесты (в памяти собранный glb, как в `tests/morphs.rs`): `a_complete_model_validates`, `missing_clips_and_bones_are_all_reported`, `names_match_case_insensitively_and_ignore_the_armature_prefix`, `an_override_wins_over_the_automatic_match`, `required_bones_come_from_layer_masks`.

## Шаг 2 — Courier на glTF (M, `redox_core` пример)
- Положить в `assets` CC-0 персонаж (**владелец решает ассет**; по умолчанию — лиса Foxfire (CC-BY, уже в репозитории): клипы Survey/Walk/Run) — **ПРОВЕРИТЬ** лицензию и атрибуцию в `assets/CREDITS` (там для лисы уже указана).
- `examples/courier/rig.rs`: `graph()` не использует процедурные клипы, а строится по именам клипов glTF (idle/walk/run + «wave» — если нет, слой руки отключён и запись в лог); `Game::new` вызывает `validate_binding` и при ошибках печатает список, откатываясь на старый процедурный rig (флаг `--procedural-rig` для сравнения).
- Скорость ходьбы из root motion берётся из клипа glTF (как в Foxfire); синхронизация фаз blend tree по маркерам/длине клипов.
- Тесты (без окна, правила Courier уже тестируются без GPU — `examples/courier/tests`): `courier_uses_the_gltf_clips_when_available`, `the_graph_plays_idle_when_standing_and_run_when_fast` (по состояниям графа `state_name`), `a_missing_wave_clip_disables_the_wave_layer_not_the_game`, `the_procedural_fallback_still_builds`. Кадровая проверка: режим `--shot` (как `foxrun`) → PNG, сравнить не пустой и отличается от процедурной версии; кадр в `golden` не добавляется (ассет внешний).

## Шаг 3 — порог CPU/GPU и автовыбор (S, `redox_render`)
- Бенчмарк `tests/gpu_skinning.rs::threshold_bench` (`#[ignore]`, release): меши 500 / 2k / 10k / 50k вершин × 1 / 10 / 100 персонажей, режимы CPU, GPU одиночный, GPU пакетный; печатать мс на кадр (`last_frame_cpu_timings`) и общее время; результат — таблица в `docs/BENCHMARKS.md`.
- `SkinningMode::{Auto, Cpu, Gpu}` в `SkinnedMeshBinding` (по умолчанию `Auto`): `Auto` выбирает GPU, если `vertex_count * morph_targets_factor ≥ GPU_SKIN_MIN_VERTICES` **или** число скинированных мешей в кадре ≥ `GPU_BATCH_MIN_MESHES`; константы берутся из таблицы замера (записать значения и дату в doc-комментарий). Чистая функция `choose_skinning_mode(vertices, meshes_in_frame, has_morphs) -> SkinningMode` с тестами.
- Тесты: `small_meshes_alone_use_the_cpu`, `many_meshes_switch_to_the_gpu_batch`, `large_meshes_use_the_gpu_even_alone`, `explicit_modes_override_auto`, `cpu_and_gpu_results_agree_for_every_mode` (регресс `gpu_skinning.rs`, `gpu_morphs.rs` зелёные).

## Шаг 4 — документация (S)
`docs/PLAN_EXECUTION.md` (A3: порог), `PROGRESS.md` (Courier на glTF), `CHANGELOG.md`, `docs/BENCHMARKS.md` (таблица порога), `README.md` примера Courier (если есть) — описание привязки и флага `--procedural-rig`.

## Границы
- **Читать:** `redox_anim/src/{graph,gltf_import,skeleton}.rs`, `redox_core/examples/{courier,foxrun}/*`, `redox_render/src/{skinning,skinning_gpu}.rs`, `tests/{gpu_skinning,gpu_morphs}.rs`.
- **Создать:** `redox_anim/src/graph_binding.rs`, тесты.
- **Менять:** `redox_anim/src/lib.rs`, `redox_core/examples/courier/{rig,game,main}.rs`, `redox_render/src/skinning.rs`.
- **Не трогать:** формат `AnimGraph`, шейдер скиннинга.

## Проверка
`cargo fmt`, `cargo clippy -p redox_anim -p redox_render -p redox_core --all-targets -- -D warnings`, `cargo test -p redox_anim`, `cargo test -p redox_core --example courier`, `cargo test -p redox_render --test gpu_skinning --test gpu_morphs`, `cargo check --workspace --all-targets`, `python scripts/check_file_sizes.py`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Ассет для Courier не добавляй без указания координатора (по умолчанию — лиса из Foxfire). Пороги в `choose_skinning_mode` бери только из таблицы замера, не угадывай.
