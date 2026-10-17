# RedOx Engine — статус

Обновлено 2026-10-17. Подробный план и журнал выполнения — `docs/ROADMAP_STAGE_0_1.md`,
история изменений — `CHANGELOG.md`, устройство — `ARCHITECTURE_REALITY.md`.

## Готово

- Этап 0 (гигиена): лицензии, линтеры (`clippy -D warnings`, `missing_docs`), `cargo deny`,
  аудит `unsafe` в ECS и Miri, интеграционные тесты, бенчмарки, ADR, документация.
- 1.1 Зависимости: wgpu 30, egui 0.36, rapier 0.36, kira 0.12, glam 0.33, winit 0.30 и др.
- 1.2 Рендер: компилятор графа кадра и декларация кадра (сверяется с кодом тестом), кэш
  пайплайнов на диске (Vulkan), проверка шейдеров при правке и пересборка 11 проходов без
  перезапуска (`Renderer::reload_shaders`), загрузка KTX2/BCn на GPU (`Texture::from_ktx2`).
- 1.3 ECS и ядро: `Commands` (в диспетчере сбрасываются между стадиями), порядок систем
  `before/after`, `run_if`, `par_for_each_mut`, `#[derive(Reflect)]`, состояния приложения с
  `OnEnter/OnExit`, хуки `on_add/on_remove`.
- 1.4 Ассеты: GUID и `.meta`, `.pak` v2 со сжатием LZ4, `AssetSource` (папка/pak), счётчики ссылок
  и выгрузка, зависимости, бюджет завершённых загрузок за кадр, KTX2/BCn, `redox_cook`
  (`pack --compress --incremental`).
- 1.5 Мир и ввод: сохранения (`SaveManager`), `world.bin` (postcard + индекс ячеек), переназначение
  клавиш, контексты ввода, геймпады (gilrs), текст и IME.
- winit: замыкание гоняется через `ApplicationHandler` (`RunClosure`), общих
  `#![allow(deprecated)]` нет.

## Не сделано

- Миграция проходов рендерера на граф кадра (граф только описывает кадр).
- Подключение `reload_shaders` к редактору/циклу приложения (API и тест на GPU есть; перезагрузка тонмаппинга и VG-проходов не реализована).
- DDGI на трассировке лучей, SDF-страницы из стриминга, GPU-скиннинг, полный граф Substrate.
- Использование `Texture::from_ktx2` материалами (загрузка на GPU есть и проверена, но ассеты материалов пока читают PNG/JPEG); стриминг текстур по приоритету.
- CI не используется (решение владельца); файлы в `.github/` — заготовка, на GitHub не запускались.
- Живая проверка геймпада и IME. Редактор и примеры запускались по 12 секунд без паник и ошибок валидации
  (после исправления падения egui 0.36), но игровая логика и вид кадра глазами не проверялись.

## Покрытие тестами

Замер `cargo llvm-cov` (2026-10-17) по десяти крейтам без GPU и UI: ecs, asset, input, scene, core, anim,
gameplay, physics, math, reflect — **80,1 % строк** (14 025 строк, 2 797 не покрыты), 79,6 % регионов.
Сильные места: ECS `world.rs` 96 %, `pak.rs` 91 %, `rebind.rs` 96 %, `state.rs` в core 97 %.
Слабые: `core/app.rs` 10 % и `window.rs` 0 % (нужен живой цикл окна), `physics/sync.rs` 20 %,
`scene/partition.rs` 61 %, `input/state.rs` 32 %, `math/matrix.rs` и `quat.rs`. Крейты рендера,
редактора, аудио, ui и vfx не измерялись: их основной код требует GPU или окна.

```bash
cargo llvm-cov --summary-only -p redox_ecs -p redox_asset -p redox_input -p redox_scene -p redox_core
```

## Проверка

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run --release --example skinned_demo -p redox_core -- --golden crates/redox_core/tests/golden
```
