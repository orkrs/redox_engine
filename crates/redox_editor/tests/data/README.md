# Данные тестов редактора (EDT-A7)

Здесь лежат только текстовые файлы. Бинарные ассеты (PNG, GLB) тесты создают сами
из кода (`tests/common/build.rs`: `make_png`, `make_glb`) и пишут вместе с `.meta`
при установке образца (`EditorHarness::install_sample`). Так в репозитории нет бинарников,
а версия формата хранится как данные.

## Состав

| Путь | Назначение |
|---|---|
| `sample_project/project.ron` | образец проекта, `format_version = 1`, стартовая сцена `scenes/main.scene.ron` |
| `sample_project/scenes/main.scene.ron` | камера, свет, пять объектов в три уровня иерархии (`Props` → `Crate` → `Crate Top`, `Lamp`), ссылки на `checker.png` и `triangle.glb` |
| `sample_project_broken/project.ron` | плагин `fancy_exporter` (загрузчика нет: `MissingPlugin`) |
| `sample_project_broken/scenes/main.scene.ron` | ссылка на отсутствующий `missing.png` (`MissingAsset`), неизвестный компонент `Gizmo3000` (значение сохраняется), ссылка на `damaged.png` |
| `sample_project_broken/assets/textures/damaged.png.meta` | повреждённый sidecar (текст, не RON) |

## Что создаёт установка образца

- `sample_project`: `assets/textures/checker.png` (GUID `…1001`) и `assets/models/triangle.glb`
  (GUID `…1002`), с `.meta`, в котором `source_hash` вычислен от байтов.
- `sample_project_broken`: `assets/textures/damaged.png` (PNG 8×8) без `.meta` (повреждённый
  `.meta` уже лежит в данных).

GUID в файлах записаны hex-строкой из 32 символов, как в `EntityGuid` и `AssetGuid`.

## Как обновить образец при смене формата

Тест `the_sample_project_opens_without_warnings` (`tests/scenarios/s00_fixtures.rs`) падает первым
и показывает, что устарело.

1. Открыть образец редактором новой версии (`File → Open Project`), ничего не менять.
2. Сохранить сцену (`Save`) и проверить, что файл отличается только форматом.
3. Заменить `scenes/main.scene.ron` и `project.ron` в `sample_project`, при необходимости
   поправить поля в `tests/common/build.rs`, если меняется формат `make_scene`.
4. Не добавлять бинарные файлы в этот каталог. Для нового ассета дописать построитель
   в `build.rs` и вызов в `install_sample`.
5. Зафиксировать изменение отдельным коммитом с пояснением, какой формат поменялся.
