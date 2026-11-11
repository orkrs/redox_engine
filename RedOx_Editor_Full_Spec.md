# RedOx Editor — полное техническое задание

**Цель:** довести RedOx Editor до уровня полноценной production-среды разработки 3D-игр, ориентируясь на рабочие процессы Unreal Engine 5 и Godot, но не копируя их архитектуру буквально.

**Статус документа:** целевая спецификация. Это не утверждение, что перечисленные функции уже существуют в RedOx.

**Главный критерий успеха:** разработчик должен иметь возможность создать проект, импортировать ассеты, собрать сцену, настроить материалы и освещение, написать/настроить поведение, запускать и отлаживать игру, профилировать производительность, управлять зависимостями и выпускать сборку — не прибегая к ручному редактированию внутренних файлов движка для обычных задач.

---

## 0. Продуктовые принципы

### 0.1. Что значит «уровень Godot / UE5»

Это не требование немедленно воспроизвести каждую отдельную систему Unreal Engine. UE5 развивает огромный набор специализированных редакторов и инструментов. Для RedOx разумная цель — сначала покрыть полный жизненный цикл обычной 3D-игры, затем расширять набор авторских инструментов.

Редактор должен быть:

- **Полноценным:** от создания проекта до упаковки игры.
- **Предсказуемым:** сохранение, импорт, компиляция, hot reload и Undo работают надёжно.
- **Неразрушающим:** операции показывают последствия, поддерживают отмену и не портят исходные ассеты.
- **Расширяемым:** панели, инспекторы, импортеры, типы ресурсов и инструменты можно добавлять через API плагинов.
- **Измеряемым:** редактор показывает, что загружено, что скомпилировано, сколько занимает память и где возникает торможение.
- **Асинхронным:** импорт, генерация миниатюр, компиляция шейдеров и cook не блокируют весь интерфейс.
- **Воспроизводимым:** один и тот же проект и настройки дают повторяемый результат сборки в поддерживаемой конфигурации.
- **Дружелюбным к исходникам:** проект имеет понятную структуру файлов, а важные данные могут храниться в diff-friendly текстовом формате.

### 0.2. Базовые правила взаимодействия

1. Любое изменение данных редактора проходит через транзакционную систему команд: Undo/Redo, группировка операций, отмена, журнал изменений.
2. У каждого ассета есть стабильный ID, не зависящий от имени файла или пути.
3. Переименование и перемещение ассета не должны ломать ссылки на него.
4. Невалидные ресурсы не должны молча исчезать: показываются ошибка, причина, путь, кнопка исправления и связанные объекты.
5. Редактор и запущенная игра должны быть разными режимами одного проекта с ясной границей editor-only/runtime.
6. UI не должен вызывать тяжёлые операции синхронно в главном потоке, если их можно выполнить в фоне.
7. Любая дорогая операция показывает прогресс, текущий этап, лог, возможность отменить безопасную часть работы и понятное сообщение об ошибке.
8. Встроенные инструменты не должны требовать от пользователя знания внутренних crate-ов RedOx для стандартных игровых задач.

---

# 1. Архитектура самого редактора

## 1.1. Отдельный редакторский слой

Разделить систему как минимум на:

- `redox_editor_app` — приложение, главное окно, меню, layout, команды и состояния UI.
- `redox_editor_core` — контекст проекта, выбор объектов, транзакции, редакторские события, undo/redo, команды.
- `redox_editor_ui` — общие виджеты, панели, дерево, property editors, диалоги.
- `redox_editor_scene` — scene document, outliner, трансформации, prefab/scene instances.
- `redox_editor_assets` — Asset Browser, реестр ресурсов, импортеры, dependency graph, миниатюры.
- `redox_editor_viewport` — 3D viewport, манипуляторы, picking, overlays, gizmos.
- `redox_editor_debugger` — запуск, инспекция runtime, логи, breakpoints, профилировщики.
- `redox_editor_build` — настройки платформ, cook, упаковка, экспорт.
- `redox_editor_plugins` — API и жизненный цикл расширений.
- отдельные специализированные crates/editor modules для материалов, мешей, анимации, аудио, VFX и других ресурсов.

Названия — рабочие, не требование буквально так назвать crates. Важнее однонаправленные зависимости и отсутствие циклической связи между UI и runtime.

## 1.2. Контекст проекта

Единый `EditorContext` должен давать доступ к:

- активному проекту и его настройкам;
- открытому документу сцены;
- выбранным объектам и ассетам;
- активному окну/viewport;
- текущему режиму (Edit / Play / Simulate / Debug);
- undo/redo stack;
- Asset Registry и dependency graph;
- импортной и build-очереди;
- системе логов и уведомлений;
- настройкам пользователя;
- source-control provider;
- панели ошибок/валидации.

Ни один редакторский плагин не должен напрямую владеть или пересоздавать глобальный runtime. Все обращения идут через публичные контексты и сервисы.

## 1.3. Отражение данных (reflection/property metadata)

Inspector не может быть написан вручную отдельно для каждого компонента. Нужна система метаданных типов и свойств:

- имя, категория, подсказка и документация;
- тип, диапазон, единицы измерения, формат;
- видимость, доступность и редактируемость;
- значение по умолчанию;
- валидатор;
- редактирование в нескольких выделенных объектах;
- сериализация/десериализация;
- миграции версий;
- ссылки на объекты и ресурсы;
- editor-only и runtime-only атрибуты;
- скрытые/расширенные настройки;
- read-only значения и вычисляемые поля.

Для пользовательских компонентов необходимо публичное API регистрации свойств. Не заставлять каждый новый компонент реализовывать собственный UI с нуля.

## 1.4. Система команд и изменений

Обязательны:

- Undo/Redo для всех типичных действий;
- транзакции с объединением нескольких низкоуровневых изменений в одну операцию;
- Undo History с просмотром последовательности операций;
- отмена длительных задач там, где это безопасно;
- маркировка документа как dirty/unsaved;
- автосохранение и восстановление после аварии;
- атомарная запись файла: не портить последний рабочий файл при выключении процесса;
- возможность сделать snapshot сцены перед опасной операцией;
- diff изменений между сохранённым состоянием и текущим.

Undo/Redo должен охватывать: transform, создание/удаление, parent/reparent, изменение свойств, ссылки на ассеты, операции с материалами, перемещение файлов редактором, изменения в поддерживаемых графах и массовые операции.

---

# 2. Главное окно и UX

## 2.1. Главное окно

Нужны:

- меню File / Edit / View / Project / Build / Tools / Window / Help;
- верхняя панель быстрых действий;
- вкладки открытых сцен и ассетов;
- центральная рабочая область;
- докируемые панели с изменением размера;
- сохранение нескольких layout-профилей;
- восстановление последнего рабочего layout после перезапуска;
- возможность отделить тяжёлый специализированный редактор в отдельное окно;
- полноэкранный/безотвлекающий режим viewport;
- контекстные меню;
- строка состояния с проектом, активной сценой, изменёнными документами, задачами импорта/build и source-control status.

### Критерии качества

- панели можно перемещать, закреплять, объединять во вкладки и восстанавливать по умолчанию;
- закрытие вкладки с несохранёнными изменениями спрашивает, сохранить ли данные;
- при потере layout или повреждённом состоянии редактор открывается в safe/default layout, а не падает;
- действия имеют команды меню, горячие клавиши и контекстные команды там, где они уместны.

## 2.2. Command Palette и поиск действий

Реализовать командную палитру по аналогии с IDE:

- быстрый вызов по hotkey;
- поиск по названию команды, ассета, объекту сцены, настройке и панели;
- fuzzy search;
- отображение горячих клавиш;
- история последних команд;
- контекстные действия для выделенного объекта;
- возможность плагина зарегистрировать команду;
- поиск и изменение настроек через палитру.

## 2.3. Горячие клавиши

- централизованная система shortcuts;
- пользовательское переназначение;
- выявление конфликтов;
- поиск команд по назначенной клавише;
- экспорт/импорт схемы;
- наборы раскладки (например, стандартная и Blender-подобная навигация);
- все основные операции доступны без мыши, если это практично.

## 2.4. Настройки редактора

Отдельно от Project Settings:

- язык, тема, масштаб интерфейса;
- шрифт и размеры UI;
- поведение viewport и камеры;
- drag-and-drop;
- автосохранение;
- число потоков фоновых задач;
- кэш и его лимиты;
- поведение импорта;
- shortcuts;
- layout;
- source control;
- уведомления и уровни логирования;
- пути к внешним инструментам;
- настройки доступности.

---

# 3. Project Manager и создание проекта

Project Manager должен уметь:

- создавать проект из шаблона;
- открывать, импортировать, дублировать, переименовывать и удалять запись проекта;
- показывать путь, версию движка, renderer/backend, последний запуск и статус;
- искать проекты по имени/пути;
- проверять совместимость версии и предлагать безопасную миграцию;
- создавать пустой проект, 3D starter, first/third person, UI demo, physics demo и rendering benchmark;
- открывать демонстрационные проекты и образцы;
- проверять отсутствующие файлы и повреждённую конфигурацию;
- создавать резервную копию перед миграцией;
- не загружать сетевой каталог без явного действия пользователя;
- опционально поддерживать Asset Library/каталог шаблонов.

Проект обязан иметь версионируемый manifest: версия формата, минимальная версия движка, активные плагины, настройки рендерера, стартовая сцена, build targets, import settings и системные требования.

---

# 4. Scene Editor / World Editor

Это центральная часть редактора.

## 4.1. Scene Outliner / hierarchy

- дерево объектов/сущностей с родительскими связями;
- поиск по имени и компонентам;
- фильтры по типу, слою, tag, visibility, locked state и наличию ошибок;
- создание, удаление, дублирование, rename;
- drag-and-drop и reparent;
- multi-select, range select, select children, select parent, invert selection;
- hide/unhide, lock/unlock, isolate selection;
- создание папок/групп;
- иконки компонентов и типов объекта;
- выделение объекта в viewport при выборе в дереве и наоборот;
- подсветка missing component/resource;
- быстрый поиск объекта из любого поля ссылки;
- массовое изменение слоя/видимости/тегов;
- виртуализация списка, чтобы дерево не тормозило на десятках тысяч объектов.

## 4.2. Scene document и сущности

- создавать и открывать scene document;
- сохранять сцену как новый файл;
- подключать вложенную сцену как reusable instance;
- создавать instance override, видеть переопределённые свойства;
- применять/сбрасывать override;
- переходить к исходной prefab/scene definition;
- локально изменять вложенные элементы с явным предупреждением о редактировании исходника;
- merge/replace сцен;
- валидировать циклические вложения;
- поиск missing references;
- сцены должны открываться детерминированно с порядком и stable IDs.

Нужны reusable prefabs/packed scenes и variants: базовый объект/сцена и набор переопределений без копирования всего содержимого. Обновление исходного prefab не должно бесконтрольно перетирать локальные overrides.

## 4.3. Transform и manipulation

- translate/rotate/scale;
- локальные и мировые координаты;
- pivot mode: origin/center/custom pivot;
- gizmos с осями и плоскостями;
- точные числовые поля transform;
- grid snapping, rotation snapping, scale snapping;
- surface snapping, vertex/pivot snapping, alignment;
- duplicate, duplicate linked/instance;
- focus selected, frame all, frame bounds;
- reset transforms;
- parent keep-world-transform / keep-local-transform;
- multi-edit и relative transform для нескольких выбранных объектов;
- зеркалирование и ограничения по оси;
- локальные gizmos компонентов (свет, камера, звук, physics shape);
- предупреждения о нулевом масштабе, отрицательном масштабе и других опасных трансформациях.

## 4.4. 3D viewport

- свободная перспектива и ортографические виды;
- presets: perspective, top, bottom, front, back, left, right;
- камера с WASD + mouse look, orbit, pan, dolly, zoom;
- управление скоростью полёта и чувствительностью;
- frame selected / frame scene;
- несколько viewport одновременно;
- независимые layout и режимы отображения каждого viewport;
- viewport camera bookmark;
- selectable overlays: grid, axes, bounds, collision, normals, tangents, skeleton, light cones, frusta, navmesh, wireframe, overdraw, meshlets, LOD, streaming cells;
- режимы Lit, Unlit, Wireframe, material preview, normals, roughness, metallic, depth, velocity, UV/debug modes;
- управление экспозицией и preview environment;
- выбор renderer debug views;
- отключение отдельных post-process эффектов;
- screenshot текущего viewport;
- показ FPS, frame time, GPU time и базового счётчика draw calls;
- gizmo для camera/light/audio/nav/physics;
- picking по GPU ID-buffer или надёжному ray picking с учётом разных типов объектов;
- прозрачное выделение объектов за стеклом/внутри групп;
- контроль editor-only визуализаций, не попадающих в игру.

## 4.5. Размещение объектов

- палитра primitives, lights, cameras, audio, physics, volumes, effects;
- drag asset из Asset Browser в сцену;
- placement preview до подтверждения;
- автоматическое выравнивание к поверхности;
- повторное размещение с разбросом масштаба/поворота;
- replace selected object with asset, сохраняя transform;
- распределение объектов вдоль spline/поверхности;
- инструменты массового размещения и brush placement;
- undo для всей операции рисования/размещения одной транзакцией.

## 4.6. Scene validation

Перед запуском/сборкой сцены редактор должен находить:

- отсутствующие ассеты и component types;
- материалы с ошибками компиляции;
- сломанные ссылки на сцены;
- отсутствующую стартовую камеру/entry point, если она требуется;
- объекты вне заданных границ мира;
- неподдерживаемые настройки renderer;
- тяжёлые текстуры, отсутствующие mipmaps, неверные форматы;
- потенциально дорогостоящие lights/shadows;
- некорректные collider/mesh settings;
- дублирующиеся имена в местах, где это запрещено;
- prefab overrides, конфликтующие с обновлённым источником.

Каждая ошибка должна иметь severity, сообщение, объект/ассет, путь, источник и доступное действие Fix/Select/Open.

---

# 5. Inspector и Property Editor

Inspector — не просто список строк со значениями.

## 5.1. Поддерживаемые типы значений

- bool, signed/unsigned integers, float/double;
- string, multiline text, localized string;
- enums/flags;
- vectors, colors, quaternion/rotation, transforms;
- ranges, curves, gradients;
- arrays, lists, maps/dictionaries;
- structs/nested objects;
- asset references;
- entity/object references;
- resource handles;
- file/folder paths;
- read-only computed values;
- custom property widgets через plugin API.

## 5.2. UX Inspector

- поиск по полям;
- категории и сворачиваемые группы;
- reset property / reset category / reset to defaults;
- copy/paste property и groups;
- copy/paste transform;
- drag-and-drop resource assignment;
- picker с фильтрацией допустимых типов;
- предупреждение при несовместимой ссылке;
- мульти-редактирование выбранных объектов;
- mixed-value state для различных значений;
- tooltips с описанием и единицами измерения;
- отображение modified-from-default;
- read-only для computed/runtime-only свойств;
- validation рядом с полем;
- advanced properties;
- pin/favorite часто используемые поля;
- возможность расширить Inspector компонентом или плагином.

## 5.3. Компоненты

- добавить/удалить компонент;
- поиск компонентов по имени и категории;
- шаблоны компонентов;
- копирование компонента между сущностями;
- paste component values без создания нового компонента, если возможно;
- предупреждение об отсутствующих зависимых компонентах;
- раскрытие runtime values при отладке;
- editor-only components не попадают в package без явного разрешения.

---

# 6. Asset Browser / Project Filesystem

## 6.1. Базовые функции

- дерево папок и режим сетки/списка;
- миниатюры и типы ассетов;
- поиск по имени, типу, тегам, пути и импортным настройкам;
- фильтры по типам, размеру, статусу, source-control state, ошибкам;
- сортировка и группировка;
- несколько вкладок и окон браузера;
- drag-and-drop в сцену, Inspector, material graph и другие редакторы;
- контекстное меню: open, rename, move, duplicate, reimport, reveal source, show references, size map, audit, delete, migrate;
- создание папок и ассетов;
- batch actions;
- отображение прогресса импорта;
- предупреждение перед удалением ассета с зависимостями;
- поиск всех ссылок до удаления;
- восстановление после случайного перемещения/переименования через Undo.

## 6.2. Asset Registry

Нужен единый реестр ресурсов с:

- стабильным UUID/GUID;
- типом и версией формата;
- source path и imported/cooked output path;
- метаданными, thumbnail и hash;
- import settings;
- dependencies и reverse dependencies;
- статусами Missing / Importing / Imported / Failed / Outdated / Cooked;
- отметкой изменения исходника;
- временем последнего импорта;
- совместимостью с текущей версией движка;
- возможностью пересканировать реестр и восстановить после аварии.

Asset Registry должен обновляться инкрементально по file watcher, но иметь полную команду Rescan/Repair. Сканирование каталога не должно приводить к повторному импорту всех ассетов без необходимости.

## 6.3. Граф зависимостей и аудит

- визуальный Reference Viewer: кто использует ресурс и на что ссылается он;
- фильтрация hard/soft dependencies, если движок поддерживает такое разделение;
- найти неиспользуемые ресурсы;
- найти дубликаты по hash и похожие файлы;
- Size Map по размеру на диске и оценке памяти;
- оценка VRAM/RAM для текстур, мешей и крупных ресурсов;
- asset audit с экспортом CSV/JSON;
- просмотр размера шейдерных вариантов и статистики cook;
- dependency closure для выбранной сцены или build target;
- миграция ссылок при перемещении/переименовании;
- отчёт о том, какие assets попадут в build и почему.

## 6.4. Форматы и pipeline импорта

Поддержать импорт через расширяемые importers, начиная с:

- 3D: glTF/GLB как предпочтительный interchange format, FBX при наличии надёжного конвертера, OBJ как простая совместимость;
- текстуры: PNG, JPEG, TGA, TIFF, HDR/EXR, KTX2/Basis, при необходимости PSD через отдельный importer;
- аудио: WAV и распространённые сжатые форматы через декодер/конвертер;
- шрифты: TTF/OTF;
- анимация/скелеты, если соответствующий runtime pipeline поддержан;
- файлы материалов, шейдеров, сцен и данных движка;
- heightmaps и наборы окружения.

Для каждого импортера:

- настройки импорта;
- preview до подтверждения;
- reimport с сохранением пользовательских overrides там, где это возможно;
- автоматический reimport при изменении source;
- журнал шагов и ошибок;
- background queue;
- cancel/retry;
- возможность задавать presets для папки/типа ассета;
- проверка лицензии/атрибуции не автоматизируется без метаданных, но для сторонних пакетов должны храниться источник и лицензия.

Не писать собственные парсеры всех индустриальных форматов без необходимости: использовать зрелые библиотеки и отделять source files от runtime/cooked assets.

## 6.5. Миниатюры и preview

- асинхронная генерация thumbnails;
- кэш превью;
- превью 3D-меша на нейтральном фоне;
- preview текстуры с checkerboard, каналами RGBA, mip levels и zoom;
- preview аудио с waveform и playback;
- preview шрифта;
- preview анимации на скелетном манекене;
- preview material на стандартных объектах/сценах;
- fallback и понятная ошибка, если thumbnail не создаётся.

---

# 7. Специализированные редакторы ресурсов

Не делать их одной огромной универсальной панелью. У каждого ресурса должен быть свой контекстный редактор, но с общими командами Save, Undo/Redo, Find References, Preview, Validation и Source Control.

## 7.1. Static Mesh Editor

- вращаемый preview и смена preview environment;
- статистика вершин, треугольников, секций, материалов, bounds;
- показ wireframe, normals, tangents, UV и collision;
- просмотр UV-каналов;
- настройка pivot/origin;
- назначение material slots;
- создание и просмотр collision shapes/convex hull;
- просмотр LOD и переходов между ними;
- генерация/настройка LOD, где поддерживается;
- настройка mesh import, scale, normals, tangents, smoothing;
- для Virtual Geometry: meshlet/clusters view, треугольники на кластер, кластеры/страницы, ошибки bake, budget и preview разных уровней детализации;
- reimport source и показ отличий;
- показатели памяти и cook size;
- проверки degenerates, invalid indices, отсутствующих UV и нормалей.

## 7.2. Texture Editor

- preview в натуральном масштабе и fit-to-window;
- zoom/pan;
- просмотр RGB/R/G/B/A, grayscale и нормалей;
- переключение mipmap levels;
- просмотр compression/format;
- import presets: base color, normal, ORM, emissive, mask, HDR;
- sRGB/linear settings;
- compression quality и max resolution;
- texture streaming settings;
- оценка размера на GPU и в памяти;
- сравнение source и imported result;
- предупреждения о normal map, alpha, слишком большой текстуре, отсутствии mipmaps.

## 7.3. Material Editor

Полноценный node graph редактор материалов:

- граф на узлах с pin-ами и соединениями;
- pan/zoom, рамочное выделение, группировка, комментарии, поиск узла;
- live preview на sphere/cube/plane/custom mesh;
- превью при разных условиях освещения;
- параметры материала: base color, metallic, roughness, normal, AO, emissive, opacity и другие поддерживаемые каналы;
- текстурные семплеры и параметрические значения;
- math, utility, procedural nodes;
- material functions/subgraphs;
- material instances/variants с переопределением параметров без копирования графа;
- сохранённые параметры и пресеты;
- проверка типов и размеров значений на соединениях;
- compile status, ошибки с переходом к узлу/строке;
- отображение сгенерированного shader code;
- статистика инструкций/вариантов и оценка стоимости;
- hot reload shader;
- поиск всех материалов, использующих конкретную текстуру/функцию;
- Undo/Redo и diff.

Важно: отдельный graph UI не имеет смысла, пока он не связан с реальным runtime material pipeline и cook. В редакторе должно быть невозможно случайно сохранить материал, который не может собраться для игры.

## 7.4. Shader Code Editor

- syntax highlighting и форматирование WGSL;
- подсветка ошибок с line/column;
- compile on demand и opt-in auto compile;
- список shader variants/permutations;
- include/import navigation;
- переход к определению и поиск использований;
- diagnostics, warnings, unused values;
- просмотр compiler output и reflection/входов/выходов;
- быстрый hot reload;
- статистика времени компиляции и кэширования;
- история изменений и diff;
- сбор shader cache в фоне;
- явный fallback при неудачной компиляции, чтобы редактор не терял предыдущую рабочую версию.

## 7.5. Skeleton / Animation Editor — по мере готовности runtime

- skeleton hierarchy и bone visualization;
- импорт и настройка skeleton/skin;
- animation clip timeline;
- воспроизведение, перемотка, loop и скорость;
- ключи и кривые трансформаций/свойств;
- animation events/notifies;
- root motion;
- blend tree/space;
- state machine, transitions, conditions и параметры;
- IK/constraints, если доступны в runtime;
- retargeting между совместимыми скелетами;
- preview персонажа с animation graph;
- отладка текущего состояния, веса blend и активных переходов;
- просмотр bone count, vertex influences и стоимости скиннинга;
- physics asset: тела, соединения, ragdoll preview;
- возможность извлечь отдельные клипы при импорте, именовать и настраивать их.

Не объявлять Animation Editor production-ready, пока импортированный скелет, клипы, blend, runtime playback и сохранение настроек проходят end-to-end тест.

## 7.6. Audio Editor / Mixer

- preview/playback любого аудиоресурса;
- waveform, trim/loop points, fades;
- import settings, sample rate, channels, compression;
- mixer graph с buses/groups;
- volume, pan, filters, EQ, reverb и поддерживаемые DSP effects;
- sound event definition: выбрать звук, рандомизацию, последовательность, параметры и правила запуска;
- spatial audio preview с источником и listener;
- attenuation curves, rolloff, occlusion settings;
- snapshots/mixer states;
- просмотр levels, peaks, clipping и voice count;
- тестирование нескольких источников без запуска игры;
- live reload аудио;
- профилировщик audio voices и CPU.

## 7.7. VFX / Particle Editor

Если RedOx планирует поддерживать полноценные эффекты, нужен редактор уровня Niagara по возможностям рабочего процесса:

- node graph эмиттеров и модулей;
- spawn/update/render stages;
- параметры, атрибуты и пользовательские переменные;
- curves и distributions;
- timeline и playback controls;
- preview scene с возможностью выбирать освещение/фон;
- CPU/GPU simulation modes, если поддерживаются;
- статистика частиц, emitters, buffers и время выполнения;
- визуализация bounds;
- hot reload;
- сохранение систем, reusable modules и presets;
- детерминированный seed для тестирования;
- ошибка компиляции/симуляции с переходом к проблемному узлу.

## 7.8. Другие редакторы — в зависимости от целей движка

- **Landscape/Terrain:** sculpt, smooth, flatten, raise/lower, heightmap import/export, layers, material painting, foliage integration, collision/navmesh refresh.
- **Foliage/Scatter:** кисть размещения, density, scale/rotation randomness, slope/height filters, erase, replace, instance statistics, culling/LOD controls.
- **Spline Editor:** создание и редактирование кривых, tangent handles, snapping, mesh/road/rail placement along spline.
- **Modeling Mode:** primitive creation, transform, merge/split, extrude, bevel, weld, normals, simple UV and collision tools. Не заменяет Blender, но закрывает небольшие правки прямо в движке.
- **Decal/Water:** визуальный preview, материал/параметры, границы и runtime cost.
- **Navigation Editor:** navmesh preview, walkable flags, costs, links, obstacles, rebuild progress, отображение ошибок и маршрутов.
- **Physics Editor:** формы коллайдеров, mass/friction/restitution, joints/constraints, debug simulation и ragdoll.
- **UI Editor:** визуальный layout, anchors, constraints, responsive scaling, hierarchy, styles/themes, fonts, input/navigation и preview на разных разрешениях.
- **2D Workspace:** только если RedOx действительно целится в широкий сегмент Godot; 2D camera, sprites, tilemaps, 2D physics, animation, UI layout. Не блокировать 3D production editor ради 2D паритета, если стратегия движка — 3D.

---

# 8. Environment / Lighting / Rendering Tools

- редактирование directional/point/spot lights и параметров теней;
- sky/atmosphere/environment map;
- fog и volumetrics, если реализованы;
- reflection probes/capture tools, если поддерживаются;
- post-processing stack и volume/zone overrides;
- camera exposure, focus/DOF, motion blur, color grading и film settings, если это поддерживает runtime;
- preview render resolution/scale;
- debug views: albedo, normals, roughness, metallic, depth, velocity, AO, shadow cascades, light clusters, overdraw, wireframe, LOD/meshlets;
- статистика по свету, теням, draw/dispatch calls и geometry;
- render graph viewer: список pass-ов, зависимости, resource reads/writes, время GPU, размеры транзиентных ресурсов, alias groups, memory peaks;
- сохранённые настройки render profile: quality tiers для разных платформ;
- сравнение двух render profiles на одной камере;
- screenshot capture с метаданными о версии проекта, разрешении и настройках;
- режимы preview для виртуальной геометрии/стриминга: cluster bounds, page residency, requests, LOD selection, eviction.

Не показывать фиктивные цифры там, где backend не способен измерить GPU time. Отмечать неподдерживаемые метрики и источник каждого значения.

---

# 9. Play / Simulate / Debug workflow

## 9.1. Режимы запуска

- **Play in Editor (PIE):** запуск игры в окне редактора или отдельном окне.
- **Simulate:** симуляция мира без полного запуска игрового entry point, если архитектура runtime это допускает.
- **Standalone:** отдельный процесс с максимально похожим поведением на собранную игру.
- **Remote target:** подключение к запущенной игре/устройству, когда это будет поддерживаться.
- **Pause / Resume / Stop / Restart / Step one frame**.
- выбор стартовой сцены и аргументов запуска;
- запуск с фиксированным разрешением/профилем;
- несколько конфигураций запуска.

## 9.2. Правила изменений во время Play

Нужно явно выбрать и документировать поведение изменений:

- изменения редакторской сцены до Play;
- runtime state, который не должен загрязнять исходную сцену;
- live-edit разрешённых свойств;
- Apply/Revert runtime changes по выбранным полям/объектам, если реализуемо;
- Stop возвращает сцену к корректному editor state;
- если runtime изменил persistent data, должно быть явное подтверждение.

## 9.3. Runtime Inspector

- осмотр сущностей/components в работающей игре;
- просмотр runtime values и resource references;
- просмотр активных систем/событий;
- поиск entity по имени/ID/type;
- выбор объекта в runtime с подсветкой в game viewport;
- изменение разрешённых значений на лету;
- отображение текущих animation states, physics values, AI/nav state, audio sources — по мере поддержки систем.

## 9.4. Console и логирование

- severity levels, категории, фильтры и поиск;
- timestamp, thread, source file/line при наличии;
- переход из log entry к объекту, ассету, скрипту или исходнику;
- copy stack trace;
- сохранение логов в файл;
- live tail для внешнего процесса;
- группировка повторяющихся сообщений;
- счётчик повторных warnings/errors;
- очищение/экспорт лога;
- уровни логирования для подсистем;
- консольные команды и автодополнение;
- history команд;
- фильтрация сообщений от editor/runtime/build/import отдельно.

## 9.5. Debugger

Если добавится scripting layer:

- breakpoints, conditional breakpoints;
- step into/over/out, continue;
- call stack;
- locals, watch expressions;
- exception/panic break;
- stack trace с source mapping;
- инспекция объектов и ECS components;
- debugger для visual scripts/graph nodes;
- отображение времени выполнения функций/узлов;
- hot reload только там, где он безопасно поддерживается.

Пока полноценной языковой/скриптовой системы нет, минимум — хорошие логи, трассировка событий, просмотр ECS и система crash reports.

---

# 10. Профилировщик и диагностика

Это обязательная часть production-ready редактора, а не приятное дополнение.

## 10.1. CPU Profiler

- frame timeline;
- main/render/worker threads;
- scopes систем и задач;
- inclusive/exclusive time;
- частота вызовов;
- распределение времени по кадрам;
- P50/P95/P99;
- frame hitch markers;
- ECS system timings, physics, animation, scripts, AI, audio, loading, UI;
- поиск самой дорогой системы;
- сравнение двух traces;
- export trace.

## 10.2. GPU Profiler

- GPU frame time;
- timing каждого render pass и compute dispatch;
- shadow rendering, depth prepass, main pass, transparency, postprocess, TAA, fog, virtual geometry;
- GPU memory / texture / buffer metrics там, где backend позволяет измерить;
- barriers/transitions и причины ожиданий, если API предоставляет эти данные;
- overdraw, fragment cost и visual debug;
- RenderDoc capture integration или экспорт совместимого capture workflow;
- сравнение traces и маркировка render passes в профиле.

Если используемый API или драйвер не раскрывает отдельную метрику — это явно показывается как unavailable, не подменяется оценкой без маркировки.

## 10.3. Asset / Memory Profiler

- список ресурсов в RAM/VRAM;
- import/cooked size;
- размеры текстур по mip/format;
- mesh/LOD/virtual geometry page memory;
- кто удерживает ссылку на ресурс;
- неиспользуемые и дублированные ассеты;
- пики загрузки и время жизни;
- выявление частых загрузок/выгрузок;
- история изменения memory budget;
- экспорт отчётов.

## 10.4. Профиль сцены

Одна панель должна показывать:

- entities/components;
- mesh instances;
- total/visible triangles;
- meshlets/clusters/pages (для Virtual Geometry);
- draw calls/indirect draws/dispatches;
- lights и shadow casters;
- текстуры и материалов в памяти;
- particles/animation/physics stats;
- размеры и границы сцены;
- предупреждения о превышении budgets.

## 10.5. Benchmark runner

- запуск заранее определённых сцен и camera paths;
- прогрев и число измеряемых кадров;
- сбор frame times и memory snapshots;
- фиксация движка, commit hash, OS, GPU, CPU, драйвера и настроек;
- экспорт JSON/CSV;
- генерация HTML/Markdown отчёта;
- сравнение с предыдущей сборкой;
- регрессии по threshold (например, P95 ухудшился на N%);
- режим headless для CI.

---

# 11. Coding, Scripting и Visual Scripting

Полноценный редактор должен давать путь от небольшого gameplay behaviour до сложной логики. Выбор языка — стратегическое решение RedOx, но редакторская инфраструктура должна поддерживать оба направления.

## 11.1. Встроенный code editor или IDE integration

- syntax highlighting;
- поиск символов и переход к определению;
- diagnostics от language server;
- quick fix и форматирование;
- поиск usages;
- open script from component;
- сборка проекта из редактора;
- build output с переходом к ошибке;
- управление external editor;
- горячие клавиши и командная палитра;
- отображение compile status;
- панель зависимостей модулей.

Для Rust-кода не нужно изобретать Rust IDE внутри движка: разумно интегрировать rust-analyzer/внешний редактор и добавить стабильную интеграцию сборки/логов. Hot reload нативного Rust-кода не обещать до выбора и проверки конкретной архитектуры.

## 11.2. Scripting runtime

Если RedOx вводит скрипты (например WASM или другой слой):

- attach/detach script component;
- exposed properties и defaults;
- lifecycle hooks;
- доступ к entities/components/events/resources через ограниченный host API;
- typed API и версии;
- ошибки исполнения с stack trace;
- лимиты времени/памяти и безопасное завершение;
- hot reload с миграцией state, если это возможно;
- упаковка скриптов и зависимостей в build;
- API docs и шаблоны;
- профилирование scripts.

## 11.3. Visual Scripting

Для уровня UE5 нужна node-based логика, хотя её можно сделать после стабильного scripting/runtime API:

- события, flow/control nodes, calls, variables, branching, loops;
- graph functions, macros/subgraphs;
- typed pins и валидация соединений;
- execution trace/highlight текущего узла;
- breakpoints, watches и step execution;
- reusable graph components;
- compile diagnostics;
- ссылки на объекты/компоненты;
- graph diff и Undo/Redo;
- версии graph schema и migrations;
- оптимизация/профилирование графов.

Нельзя начинать большой Visual Scripting Editor, пока не утверждён стабильный gameplay API и модель владения данными. Иначе UI будет постоянно ломаться при изменении runtime.

---

# 12. Level design и World Building

Для конкурентоспособного 3D-редактора должны быть предусмотрены инструменты крупного уровня, но их можно поставлять отдельными модулями.

## 12.1. Large World / World Partition

- разделение мира на cells/chunks;
- отображение grid и loaded/unloaded cells;
- загрузка выбранных областей в редакторе;
- runtime streaming sources;
- приоритеты и budgets загрузки;
- данные о размерах cells и dependencies;
- возможность редактировать одну область без загрузки всего мира;
- ссылки между cells;
- инструменты проверки границ и пропущенных ссылок.

## 12.2. Data Layers / Variants

- группы объектов по слоям;
- editor-only layers и runtime layers;
- включение/выключение видимости;
- загрузка/выгрузка слоёв;
- варианты времени суток, состояния мира, квестов и декораций;
- сравнение вариантов;
- проверка зависимостей между слоями.

## 12.3. Reusable level instances

- вложенные сцены/префабы;
- обновление instances при изменении исходника;
- local overrides;
- открытие source asset;
- отдельные overrides для transform, materials и параметров компонентов;
- проверки циклов и потерянных ссылок.

## 12.4. Работа команды над большими сценами

- отдельные файлы на уровне отдельных объектов/акторов, если это не разрушает производительность сериализации;
- стабильный порядок сериализации;
- минимизация конфликтов diff;
- ownership/locking на уровне файлов или объектов через source control;
- scene snapshots и восстановление;
- conflict viewer, показывающий изменения объектов, а не только сырой текст файла.

---

# 13. Project Settings и настройки игры

Нужны специализированные панели, а не один сырой config editor.

- Project Info: название, версия, company/product identifiers;
- стартовая сцена и entry point;
- renderer и rendering quality profiles;
- разрешение, fullscreen, VSync и frame cap;
- input actions/maps и переназначение клавиш;
- physics settings и collision layers/masks;
- layers, tags, groups;
- audio buses и настройки воспроизведения;
- localization/languages;
- save path и data locations;
- scripting/runtime settings;
- plugin management;
- asset import/cook settings;
- target platforms и build configurations;
- debug/release/development configurations;
- feature flags;
- platform-specific overrides;
- проверка устаревших/неизвестных параметров;
- diff settings между конфигурациями.

Должен быть search по настройкам, кнопка reset, документация и явное указание, где сохранится значение (editor user setting или project setting).

---

# 14. Build, Cook, Export и Release

Цель — разработчик выпускает playable build из редактора и из CLI/CI.

## 14.1. Build profiles

- платформы и архитектуры;
- debug/development/release;
- конфигурации quality settings;
- целевой renderer/backend;
- startup scene;
- window/resolution options;
- asset compression;
- shader compilation/caching;
- включение/исключение debug tools;
- channel/version/build number;
- output directory;
- environment variables/launch args;
- platform-specific settings;
- presets, которые можно хранить в репозитории.

## 14.2. Cook pipeline

- импортированные ресурсы превращаются в runtime-форматы;
- удаляются editor-only данные;
- собираются только ресурсы из dependency closure выбранной стартовой сцены/пакета;
- выполняется shader compilation для target;
- проверяются отсутствующие ресурсы и неподдерживаемые форматы;
- опционально создаются пакеты/chunks;
- кэш cook с инвалидацией по hash и версии импортера;
- отчёт по размеру и причине включения ассета;
- повторный cook не перестраивает неизменённые ассеты без причины.

## 14.3. Packaging / Export

- отдельный процесс или изолированная папка для сборки;
- прогресс и лог;
- отмена безопасных стадий;
- понятная ошибка и указание ресурса, который вызвал сбой;
- установка/запуск локального build;
- генерация build manifest;
- проверка полноты файлов;
- подпись и platform stores — по мере поддержки платформ;
- опциональные patches/deltas и DLC chunks;
- архивация отчёта вместе с build metadata.

## 14.4. CLI и CI

Все важные операции должны быть доступны без GUI:

- validate project;
- scan/import assets;
- cook assets;
- compile shaders/materials;
- run tests;
- run benchmark;
- build/package target;
- export logs/reports.

CLI обязателен для автоматизации и воспроизводимости. Редактор не должен быть единственным способом собрать игру.

---

# 15. Plugins, Extensions и Asset Library

## 15.1. Plugin API

Плагины должны уметь регистрировать:

- панели и вкладки;
- команды и hotkeys;
- контекстные меню;
- инспекторские custom property widgets;
- новые asset types и importers;
- специальные editors для ресурсов;
- инструменты viewport и gizmos;
- пункты Project Settings;
- validation rules;
- build/cook steps;
- editor events/hooks;
- автоматические тесты.

У каждого плагина нужны manifest, версия, engine API compatibility, dependencies, permissions, включение/выключение, лог загрузки и понятная ошибка при несовместимости.

## 15.2. Безопасность плагинов

- предупреждение о запуске стороннего native code;
- отображение издателя, версии, источника и permissions;
- возможность открыть проект в safe mode без сторонних plugins;
- изоляция/ограничения для скриптов, где возможно;
- plugins не должны бесконтрольно исполняться при открытии недоверенного проекта без уведомления пользователя.

## 15.3. Asset Library

- поиск/фильтры/теги;
- просмотр ассета, версий, зависимостей, лицензии и инструкции установки;
- установка/удаление/обновление пакета;
- проверка совместимости версии;
- лицензии и attribution metadata;
- offline package import;
- проектные и пользовательские коллекции;
- не требуется запускать онлайн-доступ автоматически.

---

# 16. Source Control, collaboration и snapshots

## 16.1. Source control

Начать с качественной Git-интеграции, а архитектуру сделать расширяемой для Perforce/других провайдеров.

- статус modified/added/deleted/conflicted;
- commit/push/pull/fetch через provider API или внешний инструмент;
- history и diff;
- revert/restore;
- ignore generated/cache/build files;
- подсказка о конфликте перед сохранением;
- контроль бинарных ассетов и LFS/locks, если используется;
- визуальные status icons в Asset Browser и Outliner;
- diff сцен и текстовых ресурсов;
- не коммитить кэш, временные файлы и машинно-зависимые данные без явной причины.

## 16.2. Snapshots

- снимок состояния сцены;
- сравнение трансформов/добавленных/удалённых/изменённых объектов;
- восстановление выбранных объектов;
- именованные snapshots перед крупными массовыми операциями;
- автосоздание snapshot перед migration/import batch, когда это оправданно.

## 16.3. Multi-user collaboration — продвинутая возможность

- присутствие других редакторов и активные selection;
- locking/check-out объектов/файлов;
- конфликты и их объяснение;
- live collaboration только после того, как сериализация, stable IDs, транзакции и merge-стратегия уже надёжны.

Не начинать live collaboration раньше надёжного одиночного редактирования и source control.

---

# 17. Автоматизация и тестирование

## 17.1. Editor automation framework

- unit tests для редакторских services;
- integration tests открытия/сохранения проекта;
- import tests для каждого формата;
- scene load/save roundtrip tests;
- asset dependency tests;
- тесты миграции версий;
- material/shader compile tests;
- smoke tests запуска PIE и package;
- стресс-тесты с большим числом объектов/ассетов;
- скриншотные тесты viewport с допуском на различия GPU;
- golden images для контролируемой платформы;
- тесты crash recovery;
- тесты plugin enable/disable и incompatible versions.

## 17.2. QA интерфейса

- все команды меню имеют стабильные IDs;
- критические действия доступны автоматизированным тестам;
- тестирование keyboard navigation;
- проверка high DPI/scaling;
- тестирование нескольких размеров окна;
- accessibility checks;
- regression suite при изменении формата сцены или asset registry.

## 17.3. CI

- headless project validation;
- тест открытия/сохранения sample projects;
- проверка всех демонстрационных сцен;
- сборка целевых платформ;
- render regression capture на стабильном runner;
- отчёты о производительности;
- fail build при критических regressions или broken references.

Подход соответствует зрелым редакторам, где автоматизация включает unit/feature/content stress tests и сравнение скриншотов, как описано в документации Unreal Automation Framework: https://dev.epicgames.com/documentation/unreal-engine/automation-test-framework-in-unreal-engine.

---

# 18. Интерфейс для больших проектов и производительность редактора

Редактор сам обязан оставаться быстрым на тяжёлых проектах.

- виртуализированные Outliner и Asset Browser;
- ленивое создание thumbnails;
- кэш previews;
- инкрементальное сканирование Asset Registry;
- фоновые import/cook/shader compilation;
- объединение массовых операций;
- отсутствие full tree rebuild при изменении одного объекта;
- пакетные обновления Inspector;
- ограничение частоты перерисовки неактивных вкладок;
- хранение метаданных в индексе, не обязательное полное чтение ассетов для каждого поиска;
- статистика самого редактора: UI frame time, память, количество фоновых задач;
- диагностическая панель editor performance;
- безопасное завершение и восстановление очередей после аварии.

Целевые ориентиры должны быть измеряемыми на эталонном ПК и опубликованном тестовом проекте. Не обещать универсальные цифры до тестов. Ввести бенчмарки на 10k/100k объектов, 10k/100k ассетов, большом дереве сцены и тяжёлой сцене с Virtual Geometry.

---

# 19. Документация, onboarding и UX качества

- стартовый Project Manager с понятными шаблонами;
- guided first run;
- sample projects, которые демонстрируют одну систему за раз;
- документация по каждой панели и ресурсу;
- tooltips для непонятных параметров;
- ссылки из ошибок на документацию и исправление;
- Learn/Examples panel;
- список новых функций после обновления;
- error reporting с opt-in и без отправки пользовательских данных без согласия;
- сохранение crash log и возможность открыть safe mode;
- русская и английская локализация UI как минимум в будущем, без хардкода строк в коде виджетов;
- accessibility: масштабирование интерфейса, контраст, keyboard navigation, цветовые маркеры не единственный носитель смысла.

---

# 20. Проектные форматы и совместимость

- версия формата проекта, сцены, ассета и plugin API;
- явные migrations с backups;
- проверка backwards compatibility там, где это заявлено;
- предупреждение при открытии новой версией старого проекта;
- отказ от неявного destructive upgrade;
- сохранение стабильных IDs объектов и ресурсов;
- deterministic serialization с устойчивым порядком полей;
- diff-friendly текстовое хранение редакторских документов, где это практично;
- бинарные форматы только там, где они дают разумное преимущество;
- recovery если migration прерывается;
- тесты migration на реальных sample projects.

---

# 21. Порядок реализации

Не следует разрабатывать всё одновременно. Функции сгруппированы по зависимости и ценности для повседневной разработки игр.

## Этап A — редакторская инфраструктура (блокирующий фундамент)

1. Project Manager + project manifest + шаблон пустого проекта.
2. Editor shell: dockable layout, tabs, меню, hotkeys, Command Palette.
3. Scene document format со стабильными IDs и безопасным save/load.
4. Command/transaction system, Undo/Redo, dirty state, autosave, recovery.
5. Property metadata/reflection API.
6. Asset Registry и устойчивые resource references.
7. File watcher, importer queue, error reporting.
8. Система editor/runtime boundary и editor-only data.
9. Базовые automated tests для этих механизмов.

**Definition of Done:** редактор можно закрыть/открыть без потери сцены; Undo/Redo работает для всех core actions; переименование ассета не ломает ссылки; повреждённая задача импорта не валит всё приложение; после аварии восстанавливается autosave.

## Этап B — полноценный ежедневный workflow

1. 3D viewport с navigation, transforms, snapping, picking, debug modes.
2. Scene Outliner с multi-select, hide/lock, search, layers/groups.
3. Inspector для стандартных типов данных и multi-edit.
4. Asset Browser с import/reimport, thumbnails, search, drag-and-drop.
5. Mesh Editor, Texture Editor, Material Editor, Shader Editor.
6. Play in Editor / Standalone, logs, console, runtime inspection.
7. Scene validation, missing-reference scanner, dependency viewer.
8. CLI project validation и запуск sample projects.

**Definition of Done:** простую 3D-игру с несколькими сценами, материалами, светом, физикой и входом пользователя можно собрать без ручного редактирования внутренних сериализованных файлов.

## Этап C — pipeline и production correctness

1. Cook/build profiles и воспроизводимые сборки.
2. Asset audit, Size Map, references, orphan detection.
3. Полный горячий reload поддерживаемых типов ресурсов.
4. CPU/GPU profiler и frame trace.
5. Benchmark runner, JSON/CSV/HTML reports.
6. Automated editor tests, screenshot regressions, CI.
7. Git/source-control integration.
8. Plugin API + plugin manager + safe mode.

**Definition of Done:** CI может открыть проект, проверить ссылки, импортировать ассеты, собрать игру и выполнить тесты без GUI; регрессии видны в отчёте.

## Этап D — специализированные authoring tools

1. Animation/Skeleton editor после готовности runtime animation.
2. Audio editor/mixer после стабилизации аудио API.
3. VFX/particle editor после определения particle runtime/data model.
4. Physics/navigation tools.
5. UI/2D editor по выбранной стратегии RedOx.
6. Terrain, foliage, spline, modeling tools.
7. Cinematic timeline/sequence editor и высококачественный frame capture.

**Definition of Done:** художник/дизайнер может создавать типичные ассеты и сцены внутри RedOx, а не переключаться на внешние приложения для каждой небольшой корректировки. Внешние DCC-инструменты всё равно останутся нормальной частью pipeline.

## Этап E — большой мир и командное производство

1. World partition/cell streaming tools.
2. Data layers и world variants.
3. Prefabs/scene variants и nested instances с надёжными overrides.
4. Scene snapshots и visual diff.
5. Продвинутая source-control интеграция.
6. Multi-user collaboration — только после устойчивой модели данных.
7. Asset library и расширенная система плагинов.

## Этап F — advanced parity

- Visual Scripting с отладкой;
- полноценная система cinematic Sequencer;
- продвинутые terrain/foliage/water/modeling/VFX tools;
- удалённая отладка устройств;
- профилировщики audio/network/streaming;
- расширенная platform export matrix;
- командные workflow и content review.

---

# 22. Матрица приоритетов

## P0 — нельзя выпускать Editor как полноценную среду без этого

- надёжное сохранение/загрузка сцен и проекта;
- undo/redo и autosave/recovery;
- viewport + Outliner + Inspector;
- Asset Browser + registry + import/reimport;
- stable asset IDs и разрешение ссылок;
- material/shader editing с реальным runtime compile;
- запуск игры и просмотр логов;
- scene/project validation;
- build/cook/export хотя бы на первую целевую платформу;
- автоматические smoke/integration tests;
- crash-safe операции и понятные ошибки.

## P1 — ожидаемая полноценность современного редактора

- plugin API;
- профилировщик CPU/GPU;
- asset reference graph, size map и audit;
- Git integration;
- animation/physics/navigation editors в рамках зрелости runtime;
- hot reload;
- build profiles и headless CLI/CI;
- screenshot/render regression tests;
- scene snapshots/diff;
- массовое редактирование и поиск по всему проекту.

## P2 — конкурентоспособность уровня зрелых движков

- terrain/foliage/spline/modeling modes;
- node-based VFX и audio event editing;
- UI/2D authoring;
- world partition/data layers/streaming debug;
- sequencer/cinematics/high-quality render queue;
- advanced asset library;
- визуальная отладка графов;
- remote debugging.

## P3 — дорогостоящие продвинутые возможности

- live multi-user editing;
- продвинутые collaboration/review workflow;
- визуальные node graph для всей gameplay-логики;
- расширенные cloud build/content services;
- интегрированные DCC-подобные инструменты для сложного моделирования и риггинга.

Приоритеты можно менять, но нельзя ставить P3 раньше зависимостей P0.

---

# 23. Definition of Done для редактора уровня production

Редактор нельзя считать готовым только потому, что окна выглядят хорошо. Минимальный набор приёмочных испытаний:

1. Создать проект с шаблона и повторно открыть его после перезапуска.
2. Создать сцену, добавить несколько объектов, компоненты, камеру и свет; сохранить и получить тот же результат после перезапуска.
3. Переместить, переименовать, дублировать и удалить объекты с Undo/Redo.
4. Импортировать модель и текстуры; увидеть preview, ошибки импорта и результат reimport после изменения source.
5. Переименовать/переместить ассет через редактор: все ссылки остаются целыми.
6. Найти ассеты, ссылающиеся на выбранный ресурс, и понять последствия его удаления.
7. Отредактировать материал в node graph, скомпилировать, увидеть ошибку и перейти к её источнику.
8. Запустить игру в PIE, поставить на паузу, посмотреть ECS/runtime state, увидеть логи и корректно остановить игру без загрязнения исходной сцены.
9. Получить CPU/GPU frame trace и benchmark report с метаданными оборудования.
10. Собрать release/development build из GUI и CLI.
11. Выполнить smoke/integration tests в CI без GUI.
12. Повредить или прервать одну импортную задачу — редактор продолжает работать, источник не уничтожается.
13. Принудительно завершить редактор с несохранёнными данными — после перезапуска доступно восстановление.
14. Открыть проект с отсутствующим ассетом/плагином — показать диагностический отчёт вместо необъяснимого падения.
15. Проверить большую сцену/каталог с заранее заданным размером и сохранить приемлемую отзывчивость UI.
16. Проверить масштаб UI/high DPI, переопределённые hotkeys, сохранение layout и восстановление default layout.
17. Отключить сторонний плагин и открыть проект в safe mode.
18. Проверить, что editor-only данные не попали в packaged game.

Каждая проверка должна быть автоматизирована, где это возможно, и иметь воспроизводимый тестовый проект.

---

# 24. Рекомендуемая структура рабочих пространств

Стартовый layout можно сделать таким:

- **Scene:** Viewport в центре, Outliner сверху справа, Inspector снизу справа, Asset Browser снизу.
- **Materials:** Material Graph в центре, preview слева/справа, Inspector parameters, Compile Log.
- **Animation:** preview персонажа, timeline, skeleton tree, animation graph, event/curve panels.
- **VFX:** graph, simulation preview, parameters, attribute table, profiler.
- **Audio:** event/mixer graph, waveform, preview controls, bus hierarchy, voice profiler.
- **Debug:** game viewport, entity inspector, console, CPU timeline, GPU pass timeline, memory panel.
- **Build:** target/profile selector, dependency/cook report, output log, artifact list.

Пользователь может изменять раскладку, сохранять собственные workspace presets и создавать layout для конкретной роли — programmer, technical artist, level designer, animator, sound designer.

---

# 25. Что не следует делать

1. **Не начинать с внешней красоты UI.** Сначала модель данных, сохранение, Undo/Redo и стабильные ссылки.
2. **Не строить все редакторы на жёстко прописанных структурах.** Без reflection/property metadata Inspector быстро станет тупиком.
3. **Не делать material graph без интеграции с реальным shader/material pipeline.** Это создаст красивый, но бесполезный прототип.
4. **Не обещать hot reload всего.** Для каждого типа ресурса определить контракт жизненного цикла, зависимостей и безопасной замены runtime handle.
5. **Не делать multi-user до стабильного сериализатора и diff/merge.** Иначе синхронизировать будет нечего надёжным образом.
6. **Не пытаться заменить Blender, Maya, Substance и аудиоредакторы полностью.** Редактор должен закрывать типичные задачи и интегрироваться с DCC pipeline.
7. **Не создавать огромный monolithic editor crate.** UI, asset registry, document system, viewport и специализированные редакторы должны тестироваться отдельно.
8. **Не позволять долгим импортам/компиляциям блокировать UI.** Очередь задач, прогресс, cancellation и логи проектируются сразу.
9. **Не хранить все игровые данные в редакторских типах.** Runtime не должен зависеть от egui и editor-only crates.
10. **Не гнаться за стопроцентным паритетом UE5 одним этапом.** У UE5 огромная команда и десятилетия разработки; RedOx должен наращивать инструментальную полноту по зависимостям.

---

# 26. Источники для ориентира

Список официальных документов, на которые стоит сверяться при проектировании конкретных workflow:

- Unreal Engine 5.8 — Tools and Editors: https://dev.epicgames.com/documentation/unreal-engine/tools-and-editors-in-unreal-engine
- Unreal Editor Interface: https://dev.epicgames.com/documentation/en-us/unreal-engine/unreal-editor-interface
- Unreal Level Editor: https://dev.epicgames.com/documentation/en-us/unreal-engine/level-editor-in-unreal-engine
- Unreal World Partition: https://dev.epicgames.com/documentation/unreal-engine/world-partition-in-unreal-engine
- Unreal Asset Reference Viewer: https://dev.epicgames.com/documentation/unreal-engine/reference-viewer-in-unreal-engine
- Unreal Insights / profiling: https://dev.epicgames.com/documentation/unreal-engine/introduction-to-performance-profiling-and-configuration-in-unreal-engine
- Unreal Automation Test Framework: https://dev.epicgames.com/documentation/unreal-engine/automation-test-framework-in-unreal-engine
- Unreal source control: https://dev.epicgames.com/documentation/en-us/unreal-engine/source-control-in-unreal-engine
- Godot editor overview: https://docs.godotengine.org/en/stable/getting_started/introduction/first_look_at_the_editor.html
- Godot debugger/profiler: https://docs.godotengine.org/en/latest/tutorials/scripting/debug/debugger_panel.html
- Godot AnimationTree: https://docs.godotengine.org/en/stable/tutorials/animation/animation_tree.html
- RedOx Engine repository / current README: https://github.com/orkrs/redox_engine

---

## Финальный ориентир

Редактор становится production-grade не в день, когда в нём много панелей, а когда проект можно безопасно и эффективно вести через полный цикл: **импорт → создание сцены → материалы и поведение → запуск → отладка → профилирование → тесты → cook/build → выпуск**.

Если RedOx сможет пройти этот цикл надёжно, поддерживая расширения и не блокируя работу при больших проектах, у него будет основа редактора класса Godot/UE. Дальнейшие специализированные инструменты будут усиливать продукт, а не маскировать фундаментальные пробелы.
