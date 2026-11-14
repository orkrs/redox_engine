# EDT-A8 — Редактор, hotfix этапа A: устойчивые ссылки сцены на ассеты (GUID + путь)

- **Статус:** не начат.
- **Размер:** 3 шага (M, M, S) ≈ 3 дня. **Крейты:** `redox_scene`, `redox_reflect`, `redox_render` (аксессоры), `redox_editor_scene`, `redox_editor_assets`, `redox_editor`. **Зависит от:** EDT-A2, A5, A6 (формат сцены, reflect-ссылки, реестр). **Блокирует:** снятие `#[ignore]` с трёх сценариев A7; корректность B4/B7/C1 по ссылкам.
- **Исполнение:** агент (haiku) без cargo; координатор компилирует и тестирует после каждого шага.
- Источник: `EDITOR_PLAN.md` «Открытая проблема после среза A»; `KNOWN_LIMITS.md` EDT-A-07; ТЗ §0.2.2–0.2.4, §6.2, §23.5–23.6.
- **Не входит:** миниатюры и превью импорта (B4), Reference Viewer UI (C2), cook по замыканию (C1).

## Суть
После A6/A7 сценарии переименования и обратных ссылок помечены `#[ignore]`: `MeshHandle` / `MaterialHandle` в мире хранят **индекс слота** рантайма, а не GUID и путь. Загрузчик сцены проверяет путь на `MissingAsset`, но `read_component` даёт пустую ссылку; сохранение перезаписывает `guid: null, path: null` и **стирает** ссылки. Срез вводит хранилище ссылок, которое переживает save/load и согласовано с `AssetRegistry`.

## Решение (зафиксировать в коде и отчёте)
Хранить редакторскую ссылку **рядом с handle**, не заменяя рантайм-слот:

- Компонент (или пара полей в reflect-значении) `AssetRef { guid: AssetGuid, path: String, kind: AssetKind }` в `redox_scene` (или расширение `ReflectValue::Handle` так, чтобы roundtrip сохранял `guid` + `path`).
- `MeshHandle` / `MaterialHandle` остаются индексами для GPU; при загрузке сцены редактор (или bridge) резолвит `AssetRef` → слот через `AssetService` / реестр.
- Если GUID найден, а путь изменился — путь обновляется из реестра; если GUID пропал — `LoadWarning::MissingAsset`, значение `AssetRef` **сохраняется** (не молча удаляется).

Если владелец позже выберет другое место хранения — срез всё равно обязан обеспечить roundtrip GUID+path и снять `#[ignore]` с трёх тестов A7.

## Что уже есть (проверено чтением кода / лимитов)
- `AssetRegistry` + `.meta` GUID; `redox_editor_assets::AssetService`, `ReferenceIndex`.
- `SceneDocument` / `*.scene.ron` с `LoadWarning::MissingAsset`.
- `KNOWN_LIMITS.md` EDT-A-07; сценарии A7: `the_sample_asset_reference_keeps_its_guid`, `a_forced_delete_leaves_missing_references_that_are_still_in_the_data`, `renaming_and_moving_an_asset_keeps_references_in_open_and_closed_scenes` — `#[ignore]`.

## Шаг 1 — модель ссылки и reflect (M)
- Ввести тип `AssetRef` (имя может отличаться, но поля `guid`, `path`, `kind` обязательны) с `Reflect`, RON roundtrip, `Display` GUID.
- Сериализация `MeshHandle`/`MaterialHandle` в сцене пишет/читает `AssetRef` (не голый индекс). Индекс слота в файл **не** пишется.
- Тесты (`redox_scene` / `redox_reflect` / `redox_editor_scene`): value → apply → equal; ron roundtrip; неизвестные поля игнорируются.

## Шаг 2 — загрузка, сохранение, резолв (M)
- `SceneDocument::open`: читает `AssetRef`; проверяет файл/GUID через реестр проекта (если открыт); пишет `MissingAsset` при отсутствии; **не** обнуляет ref.
- `SceneDocument::save`: пишет актуальные `guid`+`path` из компонента/`AssetRef`, не из слота.
- Мост редактора: после open / после импорта / после move ассета — резолв GUID → слот для viewport (если ассет ещё не в GPU — слот остаётся пустым, ref цел).
- `ReferenceIndex` и удаление с `force` оставляют `MissingAsset` ref в данных сцены.
- Тесты: sample scene roundtrip сохраняет GUID checker/triangle; rename/move ассета → open закрытой сцены резолвит по GUID; force-delete → ref в файле с тем же GUID и статусом missing.

## Шаг 3 — снять ignore и документы (S)
- Снять `#[ignore]` с трёх сценариев A7; поправить фикстуры sample_project при необходимости.
- `KNOWN_LIMITS.md`: EDT-A-07 закрыть или сузить; `CHANGELOG.md`; строка в `EDITOR_PLAN.md` про открытую проблему — убрать или пометить ✅.
- Отчёт `docs/engineering/notes/EDT-A8-report.md`.

## Границы
- **Читать:** `redox_render` handles, `redox_editor_scene/src/document.rs`, `redox_editor_assets`, `redox_editor/tests/scenarios/s05_*.rs`, `s06_*.rs`, `s00_fixtures.rs`, `KNOWN_LIMITS.md`.
- **Создать:** тип `AssetRef` (+ тесты), при необходимости bridge-модуль резолва.
- **Менять:** сериализацию handles в сцене, загрузчик/сейвер, сценарии A7, лимиты, changelog.
- **Не трогать:** формат OFPA, cook, UI миниатюр.

## Проверка
`cargo fmt`, `cargo clippy --workspace --lib --bins --tests -- -D warnings`, `cargo test -p redox_scene -p redox_editor_scene -p redox_editor_assets -p redox_editor --test scenarios` (и связанные); три бывших ignore — зелёные.

## Жёсткие правила исполнителя (без исключений)

Эти два правила важнее любых других строк спеки и промпта. Нарушение любого из них считается невыполнением задачи.

1. **Делай ВСЕ поставленные задачи и шаги до конца, без исключений.** Не пропускай шаги, не откладывай их «на потом», не останавливайся на полпути и не сдавай частично («остальное позже» не принимается). Если шаг неоднозначен, выбери вариант, который прямо назван в спеке (или самый простой из совместимых со спекой), запиши выбор одной строкой в отчёт и **продолжай**. Каждая строка раздела «Что сдать» и каждый тест из списка должны быть реализованы в коде. Единственное допустимое «не сделано»: шаг, который спека сама помечает как условный (с названием условия); в отчёте перечисли его и условие.
2. **АБСОЛЮТНО НИКАКИХ ПРОГОНОВ. Ничего не запускай.** Запрещено любое выполнение кода проекта и инструментов сборки: `cargo` (build, check, test, run, clippy, bench, doc, fmt), `rustc`, `rustfmt`, запуск тестов, бенчмарков и примеров, запуск собранных программ и редактора, любые замеры, скрипты, которые собирают или выполняют проект. Разрешено только: читать файлы, искать по ним, править и создавать файлы, `git` (add по явным путям, commit, status, diff, log). Компиляцию, форматирование, тесты, замеры и ручные проверки выполняет координатор по твоему отчёту. Поэтому пиши код и тесты **полностью и аккуратно с первого раза**: перечитывай собственные правки, сверяй имена и сигнатуры с исходниками, проверяй типы и заимствования глазами. Если для проверки чего-то обычно понадобился бы прогон, напиши это в отчёте списком «координатору проверить» (точные команды и что ожидается), но сам не запускай.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Ничего не запускай; коммит по явным путям; не добавляй `assets/forest`, `examples/rus_universalis`, `RedOx_Editor_Full_Spec.md`. Один шаг за заход. Отчёт `docs/engineering/notes/EDT-A8-report.md`.

## Закрытие
✅ / ⛔ / 📌. Срез закрыт, когда: save/load сохраняет GUID+path; rename/move не ломает ссылки; force-delete оставляет MissingAsset в данных; три сценария A7 зелёные без `#[ignore]`; EDT-A-07 закрыт в `KNOWN_LIMITS.md`.

## Риски
| Риск | Что делаем |
|---|---|
| Смешение слота и GUID в одном типе | Чётко: файл = AssetRef; мир GPU = Handle; bridge только в редакторе/загрузке |
| Старые сцены без guid | migrate: path-only → lookup в реестре → записать guid; иначе MissingAsset + сохранить path |
| Рендерер не знает путь из Handle | Минимальный аксессор или таблица editor-side; не тащить реестр в `redox_render` |
