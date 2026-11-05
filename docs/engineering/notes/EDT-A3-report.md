# EDT-A3 — отчёт исполнителя (шаги 1–6)

Ветка: `worktree-agent-a1e0fe41475918f78`, база: `main` (5f5f818 «docs: EDT-A1 closed»), fast-forward выполнен.
Ничего не запускалось: ни cargo, ни rustfmt, ни тесты. Код и тесты написаны и перечитаны вручную; компиляцию, форматирование и тесты выполняет координатор.

**Важно:** `EntityGuid` есть только в спеке A2, в этой ветке его нет. Код A3 написан против API из `EDT-A2.md` (см. «ПРЕДПОЛОЖЕНИЯ ОБ API»). Без слияния A2 крейт `redox_editor_core` не соберётся.

## Коммиты

| Шаг | Коммит | Содержимое |
|---|---|---|
| 1 | `ccbaba3` | feat: GUID-addressed undo commands, reflect component removal — EDT-A3 step 1 |
| 2 | `192bd47` | feat: undo transactions with labels, cancel and injectable clock — EDT-A3 step 2 |
| 3 | `b5ab2b6` | feat: undo for reparent and rename, add/remove component helpers — EDT-A3 step 3 |
| 4 | `6e115fc` | feat: dirty state from the undo history — EDT-A3 step 4 |
| 5 | `0d5c08b` | feat: record create, duplicate, delete, gizmo, brush and placement in undo — EDT-A3 step 5 |
| 6 | `fafa72a` | feat: undo history panel, goto and labelled Edit menu — EDT-A3 step 6 |
| отчёт | следующий коммит | этот файл |

## Шаг 1 — идентичность и корректность откатов

- `redox_reflect`: `Registration.remove` (`RemoveFn`, заполняется в `register::<T>()` через `remove_typed`), метод `TypeRegistry::remove_component(world, entity, type_name) -> bool`. Неизвестный тип: `false`.
- `redox_editor_core/src/guid.rs` (новый): `GuidResolver` — структура с функциями `resolve(&World, guid) -> Option<Entity>`, `guid_of(&World, entity) -> Option<EntityGuid>`, `ensure(&mut World, entity) -> EntityGuid` (сущность без GUID получает случайный GUID; правка не записывается в историю, это политика из A2).
- `undo.rs`: `UndoCommand::{SetComponent{guid,…}, Spawn{guid,snapshot}, Despawn{guid,snapshot}, Batch}`. `apply` возвращает `Result<(), UndoError>`; `UndoError::{EntityNotFound(EntityGuid), UnknownType(String), Apply(String)}`. `SetComponent` с `after: None` удаляет компонент. `Spawn` восстанавливает сущность с тем же GUID. `undo()`/`redo()` возвращают `bool`; при ошибке команда остаётся в стеке, ошибка в `UndoStack::last_error`.
- `redox_editor_core/Cargo.toml`: добавлена зависимость `redox_scene` (для `EntityGuid` и `Name`).
- `gizmo.rs` и `inspector.rs`: `SetComponent` адресуется по GUID (`GuidResolver::ensure`).
- Тесты: `redox_editor_core/tests/undo.rs` (4 теста шага 1, см. ниже), `tests/context.rs` переведён на GUID.

## Шаг 2 — транзакции и подписи

- `UndoEntry { label, command, time }`; публичные поля `undo`/`redo` заменены методами `len()`, `is_empty()`, `entries()`, `redo_entries()`, `limit()`, `can_undo()`, `can_redo()`.
- `begin(label)`, `commit() -> bool`, `cancel(world, registry) -> bool`, `in_transaction()`. Вложенные `begin` поднимают счётчик глубины; записывает одна запись с меткой внешнего вызова.
- Часы: трейт `Clock` (`now() -> Instant`), `SystemClock`, `UndoStack::with_clock(limit, Arc<dyn Clock>)`. Слияние «одного значения» идёт по окну `merge_window_secs`.
- Лимит: старые записи удаляются из начала.
- Вызовы использований полей: `redox_editor/src/app/ui.rs` (`undo.len()`). Остальные места уже работали через методы (`push`, `undo`, `redo`, `apply_hotkey`).

## Шаг 3 — недостающие команды

- `UndoCommand::Reparent { child, old_parent, new_parent, keep_world, old_local, new_local }`. Конструктор `UndoCommand::reparent(&World, child, new_parent, keep_world) -> Result<_, UndoError>` проверяет цикл и при `keep_world` пересчитывает локальный трансформ через мировые матрицы родителей.
- `UndoCommand::Rename { guid, before: Option<String>, after: Option<String> }` (компонент `Name`; `None` удаляет компонент).
- `UndoCommand::add_component(guid, type_name, value)` и `remove_component(guid, type_name, before)` — конструкторы над `SetComponent`.
- `UndoError::Cycle` добавлен.
- Тесты в `tests/undo.rs`: Reparent с `keep_world` (мировая позиция и откат, повтор), Reparent в потомка и в себя (ошибка, мир не меняется), Rename (откат и повтор).

## Шаг 4 — состояние «изменено»

- `saved_at: Option<usize>` (начально `Some(0)`): `mark_saved()`, `is_dirty()`.
- Обрезка по лимиту: если сохранённым было начальное состояние, `saved_at = None` (документ изменён до следующего сохранения).
- Новая правка, отбрасывающая ветку redo с сохранённым состоянием: `saved_at = None`.
- `EditorContext::is_dirty()` читает стек. `PIE` не трогает стек (не менялся).
- Тесты: `a_new_history_is_clean…`, `undo_back_to_the_saved_step…`, `history_cut_off_past_the_saved_state…`, `a_new_edit_after_undoing…`.

## Шаг 5 — все изменения мира через стек

- `builtin_commands.rs`: `create_entity`, `duplicate`, `delete` выполняются транзакциями «Create Entity», «Duplicate Entity», «Delete Entity». Созданная сущность записывается через публичный `record_spawn(&EditorContext, Entity)` (с GUID и снимком).
- Гизмо: транзакция открывается при успешном `start_drag` (метка «Move/Rotate/Scale Entity»), закрывается при отпускании (`end_drag`). Сама `gizmo.rs` меняется только в шаге 1.
- Кисть (`redraw.rs`): транзакция «Brush Stroke» вокруг мазка; ветка без мира сессии собирает сущности и записывает их через `record_spawn`.
- Размещение из браузера ассетов (`redraw.rs`): транзакция «Place <имя>».
- Агент StateTree (`redox_editor_ui/src/context.rs`, `spawn_demo_agent`): транзакция «Spawn StateTree Agent».
- Страж `redox_editor/tests/undo_discipline.rs`: сканирует `crates/redox_editor*/src`, запрещает `.spawn(`, `.despawn(`, `.add_component(`, `.remove_component(`, `insert_component(` вне разрешённых мест. Разрешённые файлы: `redox_editor_core/src/undo.rs`, `redox_editor_core/src/guid.rs`, `redox_editor_debugger/src/pie.rs`, `redox_editor_debugger/src/time_travel.rs`. Разрешённые строки: `// undo: tx` и `// undo: init`. Тестовый модуль (`#[cfg(test)]` до конца файла) и строки-комментарии пропускаются. Вторая тест-функция проверяет сам страж на синтетических строках.
- Тесты (`tests/undo.rs`): `create_entity` → Undo → Redo с тем же GUID; `duplicate` → один шаг, Undo убирает копию; `delete` → Undo возвращает с компонентами и GUID; мазок из 5 сущностей откатывается одним Undo.

## Шаг 6 — панель Undo History и меню Edit

- `UndoStack::goto(position, world, registry) -> bool` (шагами Undo/Redo), `UndoStack::clear()` (для кнопки «Clear»; грязность сохраняется).
- `redox_editor_ui/src/undo_history.rs` (новый): `UndoHistoryPanel::show(ui, &EditorContext)`. Строки: «Initial state», затем применённые шаги, затем отменённые (серым). Клик — `goto`. Ошибки — в консоль.
- `dock.rs`: `Tab::UndoHistory` («Undo History») в `REQUIRED_TABS`, в нижней полосе по умолчанию.
- Меню Edit (`redox_editor/src/app/ui.rs`): «Undo <метка>  Ctrl+Z» и «Redo <метка>  Ctrl+Y», неактивны без шагов; ошибки — в консоль.
- Тест: `goto_moves_to_any_step_like_sequential_undo_and_redo` (снимки мира совпадают с последовательными Undo/Redo).

## Принятые решения (одной строкой каждое)

1. `GuidResolver` — структура со статическими функциями, а не трейт; ищет сущность обходом мира (O(n)). Координатор может заменить на `GuidIndex` из A2.
2. Сущность без GUID получает случайный GUID в `GuidResolver::ensure` (не записывается в историю).
3. `undo()`/`redo()` сохраняют сигнатуру `-> bool`; ошибка в `last_error`.
4. Транзакция хранится как `Batch` даже из одной команды, чтобы обычная правка не слилась с ней по времени.
5. Внутри транзакции повторные правки одного компонента одной сущности заменяют значение «после» (одна команда на свойство, для драга). Это отступление от буквы спеки («транзакция слиянию не подлежит»): по времени слияния внутри транзакции нет, но замена по цели есть.
6. `UndoStack` больше не реализует `Debug` (из-за `Arc<dyn Clock>`); никто не требовал.
7. Отмена (`cancel`) реализована, но не подключена к интерфейсу (в спеке не требуется).
8. Иерархия: публичной функции отсоединения нет (`remove_from_children_list` приватна, ПРОВЕРИТЬ выполнено). Отсоединение реализовано в core: `remove_component::<Parent>` и правка списка `Children`.
9. Мировая матрица в Reparent считается по цепочке `Parent`; разложение через `to_scale_rotation_translation`.
10. Гизмо: транзакция в `app.rs` (вход и выход из драга), а не в `gizmo.rs`.
11. Стартовый агент: `spawn_demo_agent` записывает шаг; сразу после стартового вызова `app.rs` вызывает `mark_saved()`, поэтому редактор стартует «чистым». Стартовые кубы помечены `// undo: init` (до истории).
12. Маркер `// undo: init` добавлен сверх спеки (для стартовой сцены); спека называла только `// undo: tx`.
13. Кисть в режиме мира-сессии (OFPA, `paint_actors`) не записывается в историю: это файловая операция, ECS-спавнов там нет. Пустая транзакция не даёт записи.
14. Метка шага без явной транзакции: «Edit <тип компонента>», «Create Entity», «Delete Entity», «Reparent Entity», «Rename Entity».
15. Вкладка и меню: «Undo History», подпись и сочетание клавиш в тексте пункта Edit.
16. Строка соавтора в коммитах: `Co-Authored-By: Claude Haiku 4.5`, как в тексте задания (системная атрибуция говорила о 5.5; явная инструкция задания имеет приоритет по её же тексту).

## Места использования публичных полей стека (спека просила перечислить)

- `redox_editor/src/app/ui.rs:212`: `undo.len` → `undo.len()`. Это единственное чтение поля до шага 2.
- `redox_editor_viewport/src/gizmo.rs`, `redox_editor_ui/src/inspector.rs`, `redox_editor_core/tests/context.rs`: использовали `push`, `undo`, `redo`, `can_undo`, `SetComponent{entity}`. Переведены на GUID (шаг 1).
- `redox_editor/src/app/redraw.rs` (`apply_hotkey`), `redox_editor_ui/src/dock.rs` (`inspector`): методы, без изменений.

## ПРЕДПОЛОЖЕНИЯ ОБ API (из EDT-A2 и проверенного кода)

1. `redox_scene::components::EntityGuid` — `pub struct EntityGuid(pub u128)`, с `Clone, Copy, PartialEq, Eq, Hash, Debug` (Debug нужен `UndoCommand` и `{:?}`), `Reflect`. Компонент (blanket-реализация `Component` для `'static + Send + Sync`). Сейчас в ветке отсутствует (шаг A2 не слит).
2. `EntityGuid::new_random() -> EntityGuid` (используется в `GuidResolver::ensure`, A2 описывает «как AssetGuid::new_random»).
3. `EntityGuid` обязан быть `Copy` (код копирует его из ссылок).
4. Проверено в дереве: `redox_scene::components::Name(pub String)`; `redox_ecs::hierarchy::{Parent(pub Entity), Children{pub list}, set_parent}` публичны; `Transform{translation, rotation, scale}` с `matrix()`; `Mat4::inverse`, `to_scale_rotation_translation` (используются в `transform.rs` и `gizmo.rs`); `World::remove_component::<T>(entity) -> bool`; `Query::iter(&World)` с кортежем `(Entity, &T)`.
5. `UndoStack` в ui/дыке ссылается через `EditorContext`; `EditorLog::{info,warn,error}` принимают `AsRef<str>` (проверено).
6. `crate_layout`: `redox_editor_core` теперь зависит от `redox_scene` (не редакторский крейт, запрет не нарушен; координатор проверит).

## Известные пробелы (честно)

- **G1. Покрытие reflect.** `EditorMeshProxy`, `StateTreeComponent`, `AttributeSet`, физические и прочие компоненты не зарегистрированы в reflect. Поэтому Undo удаления, размещения и агента восстанавливает только зарегистрированные компоненты (`Transform`, `EntityGuid`, `Name`, …). Redo «Place» и «Spawn StateTree Agent» даст сущность без прокси и агента. Нужны регистрации из A2/A5/A6.
- **G2. Частичное применение.** Если вставка компонента в `Spawn` или под-команда в `Batch` падает, сущность или часть изменений остаются в мире (откат не выполняется). Ошибка видна в `last_error` и консоли.
- **G3.** Горячая клавиша Ctrl+Z/Y (`redraw.rs`, `apply_hotkey`) не пишет ошибку в консоль (только `last_error`).
- **G4.** Тест «PIE не меняет dirty» не написан (спека относит его к A7).
- **G5.** `EditorContext::is_dirty()` ещё не используется в заголовке окна (это A2 шаг 6 / A4).
- **G6.** `undo.rs` занимает 865 строк (лимит правил 800). Около 110 строк — тесты модуля. Варианты: убрать дублирующий тест `undo_redo_transform` (он есть в `tests/undo.rs`) или вынести `UndoCommand` в `undo_command.rs`. Не сделано: решение за координатором.
- **G7.** `redox_editor/tests/undo_discipline.rs` (143 строки) и `tests/undo.rs` (639 строк) в пределах лимита.

## КООРДИНАТОРУ ПРОВЕРИТЬ (по порядку)

1. Слить A2 до сборки (нужны `EntityGuid`, его путь `redox_scene::components::EntityGuid` и `EntityGuid::new_random`). Если путь или имя иные, поправить `use` в `redox_editor_core/src/{guid,undo,builtin_commands}.rs` и `tests/undo.rs`.
2. `cargo fmt --all`. Ожидаются переносы: `redox_editor/src/app/redraw.rs` (строки `w.add_component( // undo: tx`), `redox_editor_ui/src/context.rs` (`add_component( // undo: tx`), `redox_editor/tests/undo_discipline.rs` (длинная строка `let test_module`), `redox_editor_core/src/undo.rs` (`Reparent` и `reparent`), `tests/undo.rs` (импорты).
3. `cargo check --workspace --all-targets` — ожидаемые места ошибок: `EntityGuid` (путь/трейты), `Name` (путь), `redox_ecs::hierarchy` (путь), `Mat4` из `redox_math`.
4. `cargo clippy --workspace --lib --bins --tests -- -D warnings`.
5. `cargo test -p redox_reflect -p redox_editor_core -p redox_editor_viewport -p redox_editor_ui -p redox_editor`. Ожидаемо: `redox_editor_core/tests/undo.rs` 22 теста; `redox_editor/tests/undo_discipline.rs` 2 теста; `tests/context.rs` (7) и unit-тесты `undo.rs` (2) и gizmo (4) должны остаться зелёными.
6. `cargo tree -p redox_editor_core -e normal | grep egui` — пусто. `redox_scene` не должен зависеть от редакторских крейтов (проверено в его Cargo.toml: нет).
7. `python scripts/check_file_sizes.py` — `undo.rs` 865 строк (см. G6).
8. Ручной дым-тест (ТЗ §23.3): Ctrl+Shift+N, Ctrl+D, Delete, драг гизмо (одна запись «Move Entity»), мазок кисти (одна запись «Brush Stroke»), размещение ассета («Place …»), Edit → подписи Undo/Redo, вкладка Undo History: клик по шагу, Clear; create → Undo → Redo сохраняет GUID (в консоли нет ошибок). Ожидаемое отличие: G1 (redo размещения и агента без компонентов из reflect).
9. Проверить `Tab::UndoHistory` в старых layout-файлах (`ensure_tabs` добавит вкладку рядом с нижней полосой).
10. Проверить руками: `git log` для worktree — четыре файла вне A3 не трогались (`assets/forest`, `rus_universalis`, `RedOx_Editor_Full_Spec.md` — не добавлялись).

## Что не делалось (условных пунктов в спеке нет)

Все шаги 1–6 и все перечисленные тесты написаны. Не сделано только то, что спека не требует: подключение `cancel` к интерфейсу, запись кисти в режиме мира-сессии (см. решение 13), регистрация компонентов в reflect (G1, это A2/A5/A6).
