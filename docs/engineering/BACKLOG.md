# Backlog (2026-10-28)

Стабильные ID. Размеры: XS ≤ 1 ч, S ≤ полдня, M ≤ 1–2 дня (для дешёвой модели). Задачи размера L/XL здесь разбиты на цепочки.
Спецификация каждой «готовой» задачи — `specs/<ID>.md`. Состояние: **ready** (можно отдавать), **blocked** (причина указана),
**parked** (спеку не пишем: зависит от результатов этапа, устареет раньше исполнения).

## 1. Задачи со спецификациями (13)

| ID | Название | Размер | Крейт | Зависит от | Модель | Состояние |
|---|---|---|---|---|---|---|
| PHY-01 | Вертикальная скорость, гравитация, прыжок | S | redox_physics | — | haiku | ready (тест `character_v2.rs` уже в репозитории) |
| PHY-02 | Ступени, склоны, прилипание к земле | S | redox_physics | PHY-01 | haiku | ready после PHY-01 |
| PHY-03 | Подвижные платформы | M | redox_physics | PHY-01, PHY-02 | sonnet | ready после PHY-02 |
| PHY-04 | Компонент `Joint` в ECS | S | redox_physics | — | haiku | ready |
| ANM-01 | Тангенты в `SkinnedGeometry`, импорт glTF, деформация CPU | S | redox_anim | — | haiku | ready |
| RND-01 | Тангенты в CPU-скиннинге | S | redox_render | ANM-01 | haiku | ready после ANM-01 |
| RND-02 | Тангенты в GPU-скиннинге | M | redox_render | ANM-01, RND-01 | sonnet | ready после RND-01 |
| RND-03 | Категория `Buffers` в учёте памяти | XS | redox_render | — | haiku | ready |
| RND-04 | Замер CPU-времени кадра (gate A1/A2) | S | redox_render | — | haiku | ready |
| RND-05 | `Renderer::try_new` вместо паник | S | redox_render + вызовы | — | haiku | ready |
| SCR-01 | Горячая перезагрузка модуля `reload` | S | redox_script | — | haiku | ready |
| SCR-02 | SDK для авторов скриптов | M | новый крейт | SCR-01 | sonnet | **blocked**: нужна цель `wasm32-unknown-unknown` (решение владельца) |
| AUD-01 | Музыка из файла на диске | S | redox_audio | — | haiku | ready |

Тест-заготовка `crates/redox_physics/tests/character_v2.rs` (2 теста с `#[ignore]`, подтверждённо красные) живёт в коде и относится к PHY-01 и PHY-02.

## 2. Цепочки (граф зависимостей)

```
PHY-01 → PHY-02 → PHY-03
ANM-01 → RND-01 → RND-02
SCR-01 → SCR-02 (blocked)
PHY-04, RND-03, RND-04, RND-05, AUD-01 — независимы
```
Критический путь: **ANM-01 → RND-01 → RND-02** (3 шага, закрывает пункт A3 «тангенты») и **PHY-01 → PHY-02 → PHY-03** (закрывает B5).

## 3. Группы для параллельной работы (не пересекаются по файлам)

Правило проекта: в одном крейте не больше двух агентов одновременно; общие файлы (`lib.rs`, `main.rs`) назначаются одному.

| Волна | Агент 1 | Агент 2 | Агент 3 | Агент 4 |
|---|---|---|---|---|
| 1 | PHY-01 (physics) | PHY-04 (physics: `sync.rs`, `lib.rs`) | ANM-01 (anim) | AUD-01 (audio) |
| 2 | PHY-02 (physics) | RND-01 (render: `skinning.rs`) | SCR-01 (script) | RND-03 (render: `memory.rs`) |
| 3 | PHY-03 (physics) | RND-02 (render: `skinning_gpu.rs`, шейдер) | RND-04 (render: `frame.rs`) | — |
| 4 | RND-05 (render: `init.rs`, `lib.rs`, вызовы) | — | — | — |

Проверка пересечений: PHY-01/02/03 — `character.rs`, `components.rs`, `tests/character_v2.rs`; PHY-04 — `joint_component.rs`, `sync.rs`, `lib.rs`.
RND-01 — `skinning.rs`; RND-02 — `skinning_gpu.rs`, `skin_compute.wgsl`; RND-03 — `memory.rs`; RND-04 — `frame.rs`, `mod.rs` (поле); RND-05 — `init.rs`, `lib.rs`, `mod.rs` (`mod error`).
**Конфликт:** RND-04 и RND-05 оба правят `renderer/mod.rs` — не запускать одновременно (поэтому RND-05 вынесен в волну 4).

## 4. Отложено (parked) — со ссылкой на `PLAN_EXECUTION.md`

Спецификации не пишутся: каждый пункт опирается на замеры предыдущих, и точные сигнатуры за месяцы устареют.

| Пункт | Размер/цепочка при постановке | Условие запуска |
|---|---|---|
| A3 морфы (blend shapes) | M: данные в `SkinnedGeometry`, GPU-проход, glTF `targets` | после RND-02; нужен реальный ассет с морфами |
| A5 GPU-сцена и indirect | L → A5a instance-буфер, A5b compute-отсечение, A5c indirect draw | после RND-04 (если CPU кадра > 4 мс) |
| B1 горячая смена слоёв, события по слоям | S | по запросу игры |
| B3 комбинирование материалов (трение/упругость) | S | по запросу игры |
| B8 адаптивная музыка (слои по параметру) | M: данные стемов → `MusicDirector` → тест | после AUD-01 и живого прослушивания владельцем |
| SCR-03 наблюдатель файлов для `reload` | S | после SCR-01 |
| C1 TSR | L → C1a история и валидация, C1b апскейл 67 %, C1c реактивные маски | после RND-04 (бюджет CPU) |
| C2–C6 (тени, SSR, пробы, туман, DoF) | по одной на пункт | после C1 |
| D, E, F, G | по планам областей | не раньше gate A3 |
| H1–H3 навигация | L → H1a bake, H1b поиск пути, H1c избегание | после 2-дневного spike `landmass` (ADR 0007) и ответов владельца |
| UI: собственный бэкенд и текст | L | вставка между этапами |

## 5. Нужно от владельца (блокеры решений)

1. SCR-02: разрешить `rustup target add wasm32-unknown-unknown` — иначе SDK не собрать и не проверить.
2. ADR 0007 (навигация): ответы на открытые вопросы перед H1.
3. Живая проверка: звук/геймпад Courier и Foxfire, экран настроек, F5, меню и HUD Foxfire.
4. Push: `main` опережает `origin/main` на 82 коммита; без вашего «пушить» не отправляю.
