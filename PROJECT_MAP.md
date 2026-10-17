# RedOx Engine — карта проекта

> Сверено с кодом 2026-10-17 (коммит после `4bdc245`). Таблица крейтов генерируется скриптом
> `scripts/gen_crates_doc.py` в [`docs/CRATES.md`](docs/CRATES.md); здесь — то, что скрипт
> не знает: где что лежит и с чего начинать. Старая «полная карта» (wgpu 0.20, CSM, 12 крейтов)
> заменена, её можно найти в истории git до этого коммита.

## Корень репозитория

| Путь | Что это |
|---|---|
| `crates/` | 19 крейтов workspace (список и зависимости — `docs/CRATES.md`) |
| `docs/` | оценка движка, план этапов 0–1, ADR (`docs/adr/`), бенчмарки, справка по шейдерам |
| `scripts/` | `check_file_sizes.py` (лимиты размера файлов), `gen_crates_doc.py` |
| `assets/`, `worlds/arena/` | тестовые ассеты и мир в формате OFPA |
| `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`, `deny.toml` | инструментарий (Rust 1.96, MSRV 1.95) |
| `.github/` | файлы CI и шаблоны; CI на GitHub **ни разу не запускался** и пока отложен |

## Стек (Cargo.toml → `[workspace.dependencies]`)

wgpu 30 · naga 30 · winit 0.30 · glam 0.33 · egui 0.36 (+ egui_dock 0.21, egui_plot 0.37) ·
rapier3d 0.36 · kira 0.12 · image 0.25 · meshopt 0.6 · ron 0.12 · gltf 1.4 · rayon · notify 8 ·
postcard 1 · lz4_flex 0.14 · ktx2 0.5 · gilrs 0.11. Версии egui 0.37 и glam 0.34 не взяты
сознательно: под egui 0.37 нет `egui_dock`/`egui_plot`, а `kira` держит glam 0.33.

## Куда смотреть

| Задача | Файлы |
|---|---|
| Главный цикл, окно, состояния приложения | `redox_core/src/{app,window,state,commands,dispatcher}.rs` |
| Порядок систем (`before/after`, `run_if`) | `redox_core/src/dispatcher.rs` |
| ECS: мир, запросы, `Commands`, хуки компонентов | `redox_ecs/src/{world,commands}.rs`, `query/` |
| Порядок проходов кадра | `redox_render/src/renderer/frame.rs`, декларация — `redox_render/src/frame_graph.rs` |
| Компилятор графа кадра (порядок, отсечение, алиасинг) | `redox_render/src/render_graph.rs` |
| Тени (только VSM) | `redox_render/src/shadow/`, `shaders/vsm_*.wgsl` |
| Virtual Geometry | `redox_render/src/virtual_geometry/`, `shaders/vg*.wgsl` |
| Кэш пайплайнов на диске (Vulkan) | `redox_render/src/pipeline_cache.rs` |
| Ассеты: GUID, `.pak`, счётчики ссылок, зависимости | `redox_asset/src/{guid,pak,manager}.rs` |
| Упаковка ассетов (CLI) | `redox_cook` (`scan`, `pack --compress --incremental`, `list`, `verify`) |
| Ввод: действия, контексты, геймпады, текст/IME | `redox_input/src/{action,context,gamepad,text,rebind}.rs` |
| Мир и сохранения | `redox_scene/src/{ofpa,world_bin,save,partition,pcg}.rs` |
| Анимация | `redox_anim/src/{skeleton,animator,skinning,ik}.rs` |
| Редактор | `redox_editor` (бинарь), `redox_editor_core` (логика), `redox_editor_ui` (панели) |
| Примеры | `redox_core/examples/` (индекс — `examples/README.md`) |

## Команды

```bash
cargo run --release --example skinned_demo -p redox_core      # демо со скелетной анимацией
cargo run --release --example skinned_demo -p redox_core -- --golden crates/redox_core/tests/golden
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo bench -p redox_ecs -p redox_anim && cargo bench -p redox_render --bench meshlets
python scripts/gen_crates_doc.py --check
```

## Правила кода

Предупреждения clippy запрещены (`-D warnings`), `unwrap()` в библиотечном коде — предупреждение,
`missing_docs` включён в 13 крейтах, файлы длиннее 800 строк подсвечиваются
`scripts/check_file_sizes.py` (исключение: `renderer/init.rs`). Подробности — `CONTRIBUTING.md`.
