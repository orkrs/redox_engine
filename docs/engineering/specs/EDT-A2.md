# EDT-A2 — Редактор, этап A, срез A2: проект и сцена-документ (Save/Open/Save As, стабильные ID)

- **Статус:** не начат.
- **Размер:** 6 шагов (M, M, M, M, M, S) ≈ 9 дней. **Крейты:** `redox_reflect`, `redox_scene`, `redox_editor_core`, `redox_editor_scene`, `redox_editor_ui`, `redox_editor`. **Зависит от:** EDT-A1. **Блокирует:** EDT-A3 (identity), A4, A5, A6, A7.
- **Исполнение:** агент (haiku) без права запускать cargo; координатор после каждого шага компилирует и прогоняет тесты, возвращает ошибки. Шаг 6 (UI) только после зелёных шагов 1–5.
- Источник: `EDITOR_PLAN.md` §3 (D1, D4), §5 A2; ТЗ §3, §4.2, §20.
- **Не входит:** Undo для файловых операций и транзакции (A3), autosave (A4), метаданные свойств (A5), GUID-ссылки на ассеты (A6: здесь ссылки на ассеты хранятся путём), префабы и вложенные сцены (этап E), родной диалог выбора файла (в A2 путь вводится текстом).

## Суть
У редактора нет сохранения сцены: Ctrl+S в окне сохраняет только layout, `redox_reflect::register_engine_defaults` регистрирует единственный компонент (`Transform`), снимки мира не хранят идентичность сущности, а `UndoCommand::Spawn` при откате создаёт сущность с новым `Entity`. Срез вводит: покрытие reflect для компонентов сцены, стабильный `EntityGuid`, формат `*.scene.ron` с версией и отчётом о загрузке, `project.ron` (решение D4), общий тип `Level` (один файл как основной путь; OFPA-мир остаётся вторым видом, решение D1) и команды File.

## Что уже есть (проверено чтением кода)
- `redox_reflect`: `Reflect` (`type_name`, `to_value`, `apply`, `reflect_schema`), `TypeRegistry` (`register`, `snapshot_world`, `apply_snapshot`, `to_ron`, `from_ron`, `insert_component`, `read_component`), `ReflectValue` (`Null`, `Bool`, `I64`, `F64`, `String`, `List`, `Struct`), `ReflectValue::handle(type, path)` для ссылок на ассеты; `WorldSnapshot { entities: Vec<EntitySnapshot { components }> }` без идентичности сущности. `#[derive(Reflect)]` поддерживает `name` и `skip`. **Зарегистрирован только `Transform`.**
- Типы сцены без `Reflect`: `redox_scene::{Name, Tag}`, `redox_ecs::{Parent, Children}` (`set_parent`), `redox_render::{MeshHandle, MaterialHandle}` (содержат `Handle<…>`), `DirectionalLight`, `PointLight`, `SpotLight`, `Camera` (`redox_render`), физика (`RigidBody`, `Collider`, `Velocity`, … в `redox_physics`).
- `redox_scene::ofpa::write_atomic(path, bytes)`: временный файл `*.tmp.<pid>` → копия `*.bak` → `rename`. OFPA: `WorldManifest` (`version`, `name`, `cell_size`, …), `ActorRecord { id, kind, position, yaw, tags }`, `save_actor`, `save_cell_actors`.
- `redox_editor_scene::WorldSession` (после A1): открытие мира, `create_new`, запись изменённых актёров (`ofpa::save_actor`).
- Ctrl+S в `redox_editor/src/app.rs` привязан к сохранению layout (`editor_layout.ron`).

## Шаг 1 — покрытие reflect для компонентов сцены (M)
- Реализовать `Reflect` (через `#[derive(Reflect)]` там, где возможно, вручную там, где нужны преобразования) для: `Name`, `Tag`, `DirectionalLight`, `PointLight`, `SpotLight`, `Camera`, `MeshHandle`, `MaterialHandle` (значение `ReflectValue::handle(type, path)`; путь берётся из метаданных ассета: **ПРОВЕРИТЬ**, как из `Handle<MeshData>` получить путь, при отсутствии добавить в рендерер минимальный аксессор), `Parent` (хранит GUID родителя, см. шаг 2).
- Реализации лежат в крейтах, где живут типы (правило сиротства): добавить зависимость `redox_reflect` в крейты, где её нет, только при необходимости (`redox_render` уже зависит).
- Регистрация: `redox_editor_scene::register_scene_components(&mut TypeRegistry)` вызывает `register_engine_defaults` и регистрирует перечисленные типы; физика, аудио и геймплей позже (A5, B2).
- Тесты (в крейтах типов и `redox_reflect/tests`): для каждого типа `to_value` → `apply` возвращает равное значение; `ron` roundtrip по типу; неизвестные поля в значении игнорируются без паники (совместимость вперёд).

## Шаг 2 — `EntityGuid` и стабильный порядок (M)
- `redox_scene::components::EntityGuid(pub u128)` (`Clone`, `Copy`, `Eq`, `Hash`, `Reflect`: значение строка из 32 hex-символов), генерация случайная (как `AssetGuid::new_random`), печатается и разбирается без потерь.
- Индекс `GuidIndex` в `redox_editor_scene` (`HashMap<EntityGuid, Entity>` и обратный): `rebuild(&World)`, `get(guid)`, `guid_of(entity)`; поддерживается явно на операциях редактора (spawn/despawn/load), полная перестройка по запросу.
- Команды редактора `create_entity`, `duplicate` (A1) добавляют `EntityGuid` новым сущностям; `duplicate` выдаёт **новый** GUID; сущности без GUID получают его при первом сохранении (политика «назначить и отметить документ изменённым», записать в отчёт).
- `Parent` сохраняется как GUID родителя; `Children` не сохраняется и восстанавливается через `set_parent` при загрузке (второй проход после создания всех сущностей). Ссылка на отсутствующий GUID: родитель не назначается, запись в отчёт о загрузке.
- Порядок: сущности пишутся в порядке итерации мира на момент сохранения; загрузка спавнит в порядке файла; повторное сохранение без изменений даёт **те же байты**.
- Тесты (headless): GUID `Display`/`FromStr` roundtrip; индекс находит сущность после spawn и не находит после despawn; дубликат получает новый GUID; сохранение сцены с иерархией A → B → C и загрузка восстанавливают те же `Parent`/`Children` по GUID; сохранение дважды даёт идентичные байты.

## Шаг 3 — формат `*.scene.ron` и `SceneDocument` (M)
- Формат: `SceneFile { format_version: u32, name: String, entities: Vec<SceneEntity { guid, components: Vec<ReflectValue> }> }`; RON с читаемым отступом, поля в фиксированном порядке (детерминированная сериализация); версия `SCENE_FORMAT_VERSION = 1`.
- `SceneDocument` в `redox_editor_scene`: `new(name)`, `open(path, &TypeRegistry, &mut World) -> Result<LoadReport, SceneError>`, `save(&self, ..)`, `save_as(path)`; поля `path: Option<PathBuf>`, `name`, `format_version`; запись через `write_atomic` (с `.bak`).
- `SceneError` (типизированный, `thiserror`): `Io { path, source }`, `Parse { path, line, column, message }` (из позиции ошибки RON), `NewerFormat { found, supported }`, `Invalid { reason }`.
- `LoadReport { warnings: Vec<LoadWarning> }` с вариантами `UnknownComponent { entity, type_name }`, `MissingParent { entity, parent_guid }`, `MissingAsset { entity, type_name, path }` (проверка существования файла по пути ссылки относительно корня ассетов проекта, если проект открыт), `MissingGuid { entity }`.
- **Неизвестный компонент не пропадает молча** (ТЗ §0.2.4): значение сохраняется как есть в `SceneEntity.unknown_components: Vec<ReflectValue>` (поле с `#[serde(default)]`), попадает в отчёт и снова пишется при сохранении.
- Миграции: `fn migrate(file: &mut SceneFile, from: u32)` с цепочкой по версиям (в v1 пустая); при `from < SCENE_FORMAT_VERSION` перед перезаписью создаётся `*.scene.ron.bak-v<from>`.
- Тесты (`redox_editor_scene/tests/scene_document.rs`, без GPU): roundtrip «собрать сцену (Name, Tag, Transform, Parent, свет, камера, MeshHandle) → save → load в пустой мир → `snapshot_world` равны, GUID те же»; повторное сохранение даёт те же байты; повреждённый файл → `Parse` с номером строки, **исходный файл не изменён**; файл с `format_version` выше поддерживаемой → `NewerFormat`; неизвестный компонент сохраняется при пересохранении и попадает в отчёт; висящая `*.tmp.<pid>` рядом с файлом не мешает открытию; ссылка на несуществующий файл ассета даёт `MissingAsset`.

## Шаг 4 — проект `project.ron` (M)
- `ProjectManifest` (RON, `format_version = 1`): `name`, `engine_version` (версия `redox_editor_core` на момент создания), `start_scene` (путь относительно корня проекта), `assets_root` (по умолчанию `assets`), `plugins: Vec<String>`, `build_profiles: Vec<BuildProfileStub>` (пустой список, имена задел C1), `created`, `last_opened` (строки времени UTC по `std::time`, формат `YYYY-MM-DDTHH:MM:SSZ`).
- `ProjectContext` в `redox_editor_core::project`: `create(root, name, template)`, `open(root)`, `save_manifest()`, `scenes()` (все `*.scene.ron` под корнем), `recent` (список последних проектов в `editor_recent.ron` рядом с `editor_layout.ron`, максимум 10).
- Шаблоны: `empty` (пустая сцена `main.scene.ron` с камерой и направленным светом) и `3d_starter` (земля-плоскость, камера, свет, один куб); структура папок: `project.ron`, `scenes/`, `assets/`, `.redox/` (служебная, в `.gitignore`-подсказке внутри `project.ron` комментарием не писать; создать `.redox/.gitignore` с `*`).
- Совместимость: `engine_version` новее редактора → `ProjectError::NewerEngine { found, current }`; старше → предложение миграции: резервная копия `project.ron.bak-v<N>` и пересохранение; **отсутствующий** `start_scene` → отчёт, проект открывается без сцены; перечисленный в `plugins` плагин считается отсутствующим (загрузчика нет, D6): запись в отчёт `MissingPlugin { name }`, редактор не падает.
- Тесты: `create` + `open` возвращают равный манифест; шаблон `3d_starter` открывается и содержит камеру, свет и куб; манифест новее редактора → ошибка; `plugins: ["x"]` даёт `MissingPlugin` и успешное открытие; резервная копия при миграции создаётся и не перезаписывается второй раз; `recent` хранит не более 10 и убирает дубликаты.

## Шаг 5 — общий `Level` (M)
- `redox_editor_scene::Level`: `enum Level { Scene(SceneDocument), World(WorldSession) }` с общими методами `title()`, `path()`, `is_world()`, `save(&mut self, ctx)`, `is_dirty()` (до A3 dirty вычисляется по снимку: хэш снимка мира на момент сохранения против текущего; в A3 заменяется позицией в стеке Undo).
- Открытие по пути: файл `*.scene.ron` → `Scene`; папка с `world.ron` → `World` (существующий `WorldSession::open`); иначе `LevelError::Unsupported`.
- Для `World` сохранение использует существующую запись актёров OFPA (`ofpa::save_actor`), **формат OFPA и `WorldSession` не меняются**.
- Тесты: открытие `*.scene.ron` даёт `Scene`; открытие папки мира (фикстура из `redox_scene` тестов, `world.ron` плюс одна ячейка) даёт `World`; неподдерживаемый путь → `Unsupported`; `is_dirty()` меняется после правки `Transform` и сбрасывается после `save`.

## Шаг 6 — команды и UI (S)
- Команды в `CommandRegistry` (A1): `file_new`, `file_open`, `file_save`, `file_save_as`, `file_close`, `project_new`, `project_open`; `file_save` и `file_save_as` доступны только при открытом уровне.
- Меню File (верхняя панель `redox_editor/src/app/ui.rs`): New, Open, Save, Save As, Close, Open Project, New Project, Recent; Ctrl+S сохраняет уровень, **сохранение layout переходит на пункт `Save Layout` и Ctrl+Shift+S**; заголовок окна `<имя> *` при несохранённых изменениях; диалог «Сохранить изменения?» при закрытии уровня или окна (Save / Don't save / Cancel); путь вводится в текстовом поле диалога (родного файлового диалога нет).
- Диалоги и панель отчёта о загрузке (список `LoadWarning`, `MissingPlugin` и т. д.) показываются после открытия.
- Тесты: логика состояний диалога (какие кнопки доступны, когда спрашивать о сохранении) выносится в `redox_editor_core::file_state` как чистая функция и проверяется без окна; ручной дым-тест координатора: New → правка → Ctrl+S → закрыть → Open → те же данные.

## Границы
- **Читать:** `redox_reflect/src/*`, `redox_scene/src/{lib,ofpa}.rs`, `redox_ecs/src/hierarchy.rs`, `redox_editor_scene/*` (после A1), `redox_editor/src/app*`, `redox_render/src/{light,camera,systems}.rs`.
- **Создать:** реализации `Reflect` для типов сцены; `redox_scene::components::EntityGuid`; `redox_editor_scene/src/{guid_index,document,level,register}.rs`; `redox_editor_core/src/{project,file_state}.rs`; тесты `redox_editor_scene/tests/scene_document.rs`, `redox_editor_core/tests/project.rs`.
- **Менять:** `redox_editor_core/src/{context,command}.rs` (новые команды), `redox_editor_ui` (диалоги и меню), `redox_editor/src/app*`.
- **Не трогать:** формат OFPA и `world_bin`, `UndoStack` (кроме чтения), рендерер (кроме аксессора пути ассета), другие редакторские крейты.

## Проверка
`cargo fmt`, `cargo clippy --workspace --lib --bins --tests -- -D warnings`, `cargo test -p redox_reflect -p redox_scene -p redox_editor_scene -p redox_editor_core -p redox_editor_ui`, `python scripts/check_file_sizes.py`, `python scripts/gen_crates_doc.py --check`; ручной дым-тест (координатор): создать проект из шаблона `3d_starter`, изменить объект, сохранить, закрыть редактор, открыть снова, убедиться в тех же данных (ТЗ §23.1–23.2).

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Не запускай cargo; форматируй только свои файлы (`rustfmt --edition 2024 <файл>`); коммит по явным путям, без `git add -A`; не добавляй `assets/forest`, `examples/rus_universalis`, `RedOx_Editor_Full_Spec.md`. Один шаг за заход. Отчёт `docs/engineering/notes/EDT-A2-report.md` (файлы, команды проверки, отклонения от спеки, вопросы).

## Закрытие
✅ / ⛔ / 📌 по шагам. Срез закрыт, когда: сцена из шаблона сохраняется и открывается с равным снимком и теми же GUID и иерархией; повторное сохранение даёт те же байты; повреждённый файл не портит предыдущую версию; неизвестные компоненты не теряются; `project.ron` создаётся, открывается и мигрируется с резервной копией; `Level` открывает оба вида уровня; Ctrl+S сохраняет уровень; тесты зелёные.

## Риски
| Риск | Что делаем |
|---|---|
| `Handle<MeshData>` не даёт путь ассета | Проверить в шаге 1; при необходимости минимальный аксессор в рендерере; иначе ссылки на ассеты в A2 хранятся только для типов с известным путём, остальное 📌 |
| Детерминированность RON при сохранении | Сортировка не используется (порядок мира), поля структур фиксированы; тест «те же байты» ловит расхождения |
| Подсчёт `is_dirty` по хэшу снимка дорог | Допустим до A3; замер на 10 000 сущностей в A7, замена на позицию стека |
| Миграции без реальных старых файлов | В тесте используется искусственная миграция v0 → v1; реальные начнутся после первой смены формата |
