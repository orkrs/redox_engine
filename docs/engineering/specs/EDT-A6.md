# EDT-A6 — Редактор, этап A, срез A6: реестр ассетов в редакторе (ссылки по GUID, перемещение, очередь импорта)

- **Статус:** не начат.
- **Размер:** 7 шагов (M, M, M, M, M, M, S) ≈ 10 дней. **Крейты:** `redox_asset`, `redox_reflect`, `redox_scene`, `redox_editor_core`, `redox_editor_assets`, `redox_editor_scene`, `redox_editor_ui`, `redox_editor`. **Зависит от:** EDT-A1, A2 (формат сцены), A3 (команды), A5 (поле ссылки). **Блокирует:** A7 (сценарии переименования, ссылок, сбоя импорта); B4 (импорт моделей).
- **Исполнение:** агент (haiku) без cargo; координатор компилирует и тестирует после шагов. Шаги 1–2 меняют `redox_asset` (общий крейт): полный прогон `cargo test --workspace` после них.
- Источник: `EDITOR_PLAN.md` §5 A6; ТЗ §6.1–6.4, §23.5–23.6, §23.12.
- **Не входит:** миниатюры и предпросмотры (B4), реальные импортёры форматов и cook (B4, C1; здесь каркас и два проверяющих импортёра), граф зависимостей как визуализация и аудит (C2), поиск дубликатов по хэшу как отчёт (C2), Undo для действий с файлами на диске при внешних изменениях.

## Суть
Движок уже умеет GUID-реестр ассетов (`.meta` рядом с файлом, `move_asset`, `remove_asset`), но редактор его не использует: браузер ассетов показывает файлы по путям, компоненты ссылаются на ассеты строкой-путём, при перемещении файла ссылки ломаются, импорта в фоне нет, наблюдатель файлов видит только события изменения. Срез делает GUID основой ссылок и подключает реестр, наблюдатель и очередь импорта к редактору так, чтобы сбой одной задачи не влиял на остальные.

## Что уже есть (проверено чтением кода)
- `redox_asset/src/guid.rs`: `AssetGuid(u128)` (Display/FromStr, serde как строка), `AssetMeta { guid, importer_version, source_hash, settings }` в `*.meta` (RON), `content_hash` (FNV-1a), `AssetRegistry { root, by_guid, by_path }` с `scan(root) -> (Self, ScanReport)` (создаёт недостающие sidecar, чинит дубликаты GUID, повреждённые sidecar попадают в `damaged`), `path_of`, `guid_of`, `iter`, `move_asset(from, to)`, `remove_asset(path)`, `is_modified(path)`, `mark_imported(path, importer_version)`; `RegistryError::{Io, BadMeta, Unknown, Occupied, …}`.
- `redox_asset/src/hot_reload.rs`: `HotReloadWatcher` (`notify`), `watch`, `unwatch`, `drain_reload_requests() -> Vec<ReloadRequest>`; **реагирует только на `EventKind::Modify`** (создание, удаление и переименование не отдаёт).
- `redox_asset/src/manager.rs` и `queue.rs`: менеджер загрузки с очередью приоритетов (для рантайма, не для импорта в редакторе).
- `ReflectValue::handle(type_name, Option<path>)` хранит ссылку как структуру с полем `path`; `as_handle_path()` читает его.
- `AssetBrowserPanel` (`redox_editor_ui/src/asset_browser.rs`): список файлов по папке, `AssetKind::from_path`, быстрые пробы текстур (`probe_texture`) и glTF; кнопка Spawn создаёт `AssetSpawnRequest { path }`.
- В `redox_editor*` нет использований `AssetRegistry` и `HotReloadWatcher`.

## Шаг 1 — ссылки по GUID в reflect и сценах (M)
- Значение ссылки на ассет: `Struct { type_name, fields: [("guid", String(hex32) | Null), ("path", String | Null)] }`; `path` остаётся подсказкой/запасным вариантом, **источником истины является `guid`**. Новые функции: `ReflectValue::asset_ref(type_name, guid: Option<AssetGuid-строка>, path: Option<String>)`, `as_asset_guid()`; `handle(...)` и `as_handle_path()` остаются для совместимости (обёртки).
- При загрузке сцены: если есть `guid` и он известен реестру — путь берётся из реестра (подсказка `path` обновляется), если неизвестен, но есть `path` и файл существует — GUID восстанавливается по пути реестра и запись в отчёт `LoadWarning::AssetGuidRestored`; если нет ни того ни другого — `MissingAsset`.
- При сохранении сцены `path` в ссылке записывается актуальным относительно корня ассетов.
- Старые сцены (только `path`) читаются без потерь (тест).
- Типы `MeshHandle`, `MaterialHandle` (A2) читают и пишут новую форму.
- Тесты: сохранение сцены с ссылками пишет `guid` и `path`; загрузка по GUID находит файл, переименованный после сохранения; загрузка старого формата (только `path`) восстанавливает GUID; ссылка на пропавший файл даёт `MissingAsset` и не удаляется из данных.

## Шаг 2 — расширение реестра и наблюдателя (M)
- `AssetRegistry`: статусы `AssetStatus { Missing, Importing, Imported, Failed(String), Outdated }` (хранятся в памяти и пересчитываются по `source_hash` и `importer_version`: `Outdated`, если хэш источника отличается от записанного в `.meta`), `status(path)`, `set_status`.
- Обратные ссылки: `AssetRegistry::dependents(guid) -> Vec<DependentRef>` строится сканером `ReferenceIndex` (в `redox_editor_assets`): обходит `*.scene.ron` проекта и ищет ссылки по GUID; индекс обновляется инкрементально при сохранении сцены и при изменении файла сцены; команда Rescan References пересобирает целиком.
- `HotReloadWatcher`: `drain_events() -> Vec<AssetEvent>` с вариантами `Created(path)`, `Modified(path)`, `Removed(path)`, `Renamed{from,to}`; `drain_reload_requests` остаётся (обёртка над `Modified`) для совместимости. События `.meta` и временных файлов (`*.tmp.*`, `*.bak`) фильтруются.
- `AssetRegistry::apply_event(event) -> RegistryChange` обновляет реестр **инкрементально**: `Created` — новый sidecar при необходимости; `Removed` — `Missing` (запись GUID сохраняется, пока на неё есть ссылки); `Renamed` — перенос записи с сохранением GUID, если sidecar переехал вместе с файлом; `Modified` — пересчёт `Outdated`.
- Команда Rescan/Repair вызывает `scan` и возвращает `ScanReport`; **полный повторный импорт не запускается**: только файлы со статусом `Outdated` или без импортированной версии.
- Тесты (`redox_asset/tests/registry_events.rs`): создание файла даёт sidecar и GUID; переименование снаружи (файл вместе с `.meta`) сохраняет GUID; удаление даёт `Missing`; изменение содержимого даёт `Outdated`; `.tmp`/`.bak` игнорируются; `dependents` находит сцены, ссылающиеся на GUID.

## Шаг 3 — сервис ассетов редактора (M)
- `redox_editor_assets::AssetService`: владеет `AssetRegistry`, `HotReloadWatcher`, `ReferenceIndex`, очередью импорта; методы `open(project)`, `tick(&mut self) -> Vec<AssetChange>` (вызывается главным циклом: забирает события, применяет к реестру, ставит нужные импорты), `rescan()`, `resolve(guid) -> Option<&str>`, `guid_of(path)`, `status(path)`, `dependents(guid)`.
- Перемещение, переименование и удаление идут через сервис: `move_asset(from, to) -> Result<MoveReport>` выполняет `registry.move_asset`, обновляет пути-подсказки в сценах проекта (открытая сцена: через команды Undo A3 `SetAssetRef`, закрытые файлы сцен: перезапись `path` с атомарной записью после подтверждения), возвращает отчёт; `delete_asset(path)` **не удаляет**, если есть зависимые, пока не передан `force`; возвращает список зависимых сцен и сущностей для диалога подтверждения.
- Undo для переноса: `UndoCommand::MoveAsset { from, to }` (A3 резервировал имя) откатывает перенос; для `delete_asset(force)` Undo недоступен, диалог предупреждает (ТЗ §23.6).
- Тесты (`redox_editor_assets/tests/service.rs`, временный каталог проекта): перемещение ассета не ломает ссылку в открытой и закрытой сценах (загрузка после переноса разрешает ссылку по GUID); `delete_asset` без `force` возвращает зависимых; с `force` удаляет и ссылки становятся `MissingAsset` (не пропадают); внешнее переименование обнаруживается `tick` и обновляет реестр; `Undo` переноса возвращает файл и пути.

## Шаг 4 — очередь импорта и каркас импортёров (M)
- `redox_editor_assets::import`: `trait AssetImporter: Send + Sync { fn name(&self) -> &str; fn extensions(&self) -> &[&str]; fn version(&self) -> u32; fn import(&self, job: &ImportJob, cancel: &CancelToken) -> Result<ImportOutput, ImportError>; }`; `ImportJob { path, guid, settings }`; `ImportOutput { report: ImportReport }` (предупреждения и сведения: размеры, число мешей/текстур).
- `ImportQueue`: пул рабочих потоков (по умолчанию `min(cores - 1, 4)`, настройка в `EditorSettings`), приоритеты (открытый сейчас ассет выше), прогресс, отмена (`CancelToken`), повтор (`retry(path)`), события `ImportEvent::{Started, Progress, Finished, Failed}`; состояние пишется в статус реестра (`Importing` → `Imported` / `Failed(сообщение)`); после успеха вызывается `mark_imported(path, importer.version())`.
- **Изоляция ошибок:** каждый импорт выполняется под `catch_unwind` с логированием; паника или `Err` одной задачи даёт `Failed` с понятным сообщением, остальные задачи и редактор продолжают работу; исходный файл не изменяется ни при каких исходах.
- Каркас реестра импортёров (`ImporterRegistry`), в A6 два импортёра-проверки: `GltfValidateImporter` (загружает файл `redox_asset` загрузчиком и составляет отчёт: число мешей, материалов, текстур, предупреждения), `TextureProbeImporter` (разбор заголовка PNG/JPEG/KTX2, размеры и формат); реальный импорт и cook в B4/C1.
- Тесты (`redox_editor_assets/tests/import_queue.rs`, фикстуры собираются в тесте): корректный файл → `Imported`; битый файл → `Failed` с сообщением, статус остальных `Imported`; импортёр, паникующий в тесте, не роняет очередь; отмена до старта и во время работы; повтор после исправления файла даёт `Imported`; приоритет открытого ассета; `Outdated` после изменения файла запускает повторный импорт только этого файла.

## Шаг 5 — браузер ассетов на реестре (M)
- `AssetBrowserPanel` показывает записи реестра (имя, тип, статус с цветом, GUID в подсказке), а не просто файлы; фильтры по типу и статусу, поиск по имени и пути; контекстное меню: Rename, Move, Delete (с диалогом зависимых), Reimport, Reveal in Explorer, Show References (список сцен и сущностей), Copy GUID; перетаскивание ассета на поле ссылки в Inspector назначает ссылку по GUID (A5, шаг 6).
- Статус-индикаторы: `Importing` с прогрессом, `Failed` с кнопкой «Показать ошибку» (панель отчёта с путём и сообщением, ТЗ §0.2.4), `Outdated`.
- Кнопка Spawn по-прежнему создаёт объект, но запись создаётся транзакцией (A3) и ссылка хранится по GUID.
- Логика состояния панели (фильтры, выбор, список действий для записи) вынесена в `redox_editor_assets::browser_model` без egui и проверяется тестами.
- Тесты (на модели): фильтр по статусу и типу; поиск; набор допустимых действий для `Failed` и `Missing`; Show References возвращает ожидаемые сцены.

## Шаг 6 — открытие проекта с проблемными ассетами (M)
- При открытии проекта сервис ассетов сканирует корень, формирует `AssetReport { created, regenerated, damaged, missing_refs }` и показывает панель диагностики (ТЗ §23.14): список с severity, путём, источником (сцена/сущность/компонент) и действиями Select/Open/Fix (Fix: «переназначить» или «удалить ссылку»).
- Повреждённый `.meta` не роняет открытие (запись в `damaged`, файл исключён из реестра, действие «пересоздать sidecar»); дубликат GUID, возникший при копировании файла вместе с sidecar, получает новый GUID (поведение реестра) с записью в отчёт.
- Тесты: проект с отсутствующим файлом, на который ссылается сцена, открывается и даёт `MissingAsset` в отчёте; повреждённый `.meta` попадает в `damaged`; копия файла с sidecar получает новый GUID и исходная ссылка остаётся на исходном.

## Шаг 7 — интеграция с главным циклом и настройки (S)
- `AssetService::tick` вызывается главным циклом; изменения реестра обновляют браузер и (через события) помечают сцену `dirty`, если путь-подсказка в открытой сцене изменился.
- `EditorSettings` (A4): число рабочих потоков импорта и автоматический импорт при изменении файла (вкл/выкл).
- Строка состояния: «Importing N / Failed M».
- Ручной дым-тест (координатор): положить новую текстуру в `assets/`, увидеть её в браузере со статусом, переименовать в браузере, убедиться, что ссылка в сцене цела; испортить файл glTF и увидеть `Failed` без падения.

## Границы
- **Читать:** `redox_asset/src/{guid,hot_reload,manager,queue}.rs`, `redox_asset/src/loader/gltf_loader.rs`, `redox_editor_ui/src/asset_browser.rs`, `redox_editor_scene/src/document.rs` (A2), `redox_editor_core/src/{undo,context,settings}.rs`, `redox_reflect/src/lib.rs`.
- **Создать:** `redox_editor_assets/src/{service,reference_index,import,browser_model}.rs`, `redox_asset/tests/registry_events.rs`, `redox_editor_assets/tests/{service,import_queue}.rs`.
- **Менять:** `redox_asset/src/{guid,hot_reload}.rs`, `redox_reflect/src/lib.rs` (ссылки по GUID), `redox_scene` (поля/функции для ссылок), `redox_editor_ui/src/asset_browser.rs`, `redox_editor_core/src/undo.rs` (`MoveAsset`), `app.rs` (вызов `tick`).
- **Не трогать:** `AssetManager` рантайма и его очередь, `redox_cook`, форматы `.pak` и загрузчики (кроме чтения).

## Проверка
`cargo fmt`, `cargo clippy --workspace --lib --bins --tests -- -D warnings`, `cargo test --workspace` (шаги 1–2 меняют общие крейты), `check_file_sizes.py`, `gen_crates_doc.py --check`; ручной дым-тест (координатор; ТЗ §23.4–23.6, §23.12).

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Не запускай cargo; форматируй только свои файлы; коммит по явным путям; не добавляй `assets/forest`, `examples/rus_universalis`, `RedOx_Editor_Full_Spec.md`. Один шаг за заход; шаги 1 и 2 сдавать отдельно. Отчёт `docs/engineering/notes/EDT-A6-report.md`.

## Закрытие
✅ / ⛔ / 📌 по шагам. Срез закрыт, когда: ссылки в сценах хранятся по GUID и переживают перемещение и переименование; удаление ассета предупреждает о зависимых; наблюдатель обновляет реестр инкрементально; импорт идёт в фоне с прогрессом, отменой и повтором, сбой одной задачи не влияет на остальные и не трогает источник; проект с повреждёнными ассетами открывается с отчётом; тесты зелёные.

## Риски
| Риск | Что делаем |
|---|---|
| События `notify` на Windows приходят пачками и дублируются (переименование как Remove+Create) | Дебаунс 150 мс и склейка пар Remove+Create с совпадающим sidecar/хэшем в `Renamed`; тесты на пачки |
| Перезапись путей в закрытых сценах опасна | Только после подтверждения и атомарно, с резервной копией; обновляется только подсказка `path`, GUID не меняется |
| `catch_unwind` не ловит abort | Импортёры не используют `panic = "abort"`; в отчёте указать профиль; тяжёлые импортёры позже выносятся в процесс |
| Индекс обратных ссылок устаревает | Пересборка по команде и при сохранении сцены; тест на устаревание после внешнего изменения файла сцены |
| Размер `asset_browser.rs` и сервиса | Разделение на подмодули заранее (`service`, `index`, `queue`, `model`) |
