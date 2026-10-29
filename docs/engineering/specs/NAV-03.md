# NAV-03 — Навигация: `NavWorld` (поиск пути, запросы), off-mesh связи, ECS-компоненты, отладка

- **Статус:** выполнено 2026-10-29.

- **Размер:** M + M + S (шаги 1–3). **Крейт:** `redox_nav` (шаги 1–2), `redox_core` (шаг 3: подключение, 1 файл). **Зависит от:** NAV-02 (`NavData`, `.nav`), NAV-01 (факты `landmass`). **Блокирует:** NAV-04.
- **Модель:** haiku.
- Допущения владельца — в NAV-01 (один класс агента; API уже принимает `AgentClass(u8)`; off-mesh связи из кода/файла сцены).

## Суть
Пользователь движка должен вызвать `nav.find_path(from, to)` и получить список точек, а агентам в ECS — выставить цель и увидеть, что они идут. Алгоритмы берёт `landmass` (A*, упрощение пути, off-mesh); снаружи — наш фасад.

## Шаг 1 — фасад `NavWorld` (M)
```rust
pub struct NavWorld { /* landmass::Archipelago + таблица плиток + стоимости зон */ }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct AgentClass(pub u8);

impl NavWorld {
    /// Builds a world from baked data. Every tile becomes one landmass island. Area costs default to 1.0.
    pub fn from_data(data: &NavData) -> Result<Self, NavError>;
    /// Cost multiplier of an area (>= 1.0 makes it more expensive; `f32::INFINITY` is treated like NOT_WALKABLE). NaN/negative -> `NavError::BadCost`.
    pub fn set_area_cost(&mut self, area: NavAreaId, cost: f32) -> Result<(), NavError>;
    /// Replaces the tile at `coord` (runtime change of one tile). The rest stays.
    pub fn replace_tile(&mut self, tile: NavTile) -> Result<(), NavError>;
    /// Removes the tile at `coord`; returns whether it existed.
    pub fn remove_tile(&mut self, coord: (i32, i32)) -> bool;

    /// Shortest path as waypoints (including `from` snapped to the mesh and `to` snapped), or `None` when there is none.
    pub fn find_path(&self, from: Vec3, to: Vec3, class: AgentClass) -> Option<Vec<Vec3>>;
    /// Nearest point of the mesh within `max_distance`, or `None`.
    pub fn nearest_point(&self, p: Vec3, max_distance: f32) -> Option<Vec3>;
    /// Straight-line walk along the mesh: the first point where the segment leaves it, or `None` when the whole segment lies on the mesh.
    pub fn raycast(&self, from: Vec3, to: Vec3) -> Option<Vec3>;
    /// A random point on the mesh within `radius` of `center` (uniform by area), using the caller's generator.
    pub fn random_point_in_radius(&self, center: Vec3, radius: f32, rng: &mut impl FnMut() -> f32) -> Option<Vec3>;
}
```
`rng` — функция, возвращающая число в [0,1) (в движке есть `SplitMix64` в `redox_audio::cues`; в `redox_nav` свой не вводить, принимать замыкание).
Типы наружу — `redox_math::Vec3` (glam 0.33); конвертация `[f32;3]` ↔ foreign glam только внутри `backend.rs`.
Тесты (`tests/paths.rs`; карты строятся прямо в тесте через `NavData`, без запекания: прямоугольные полигоны):
```rust
#[test] fn a_straight_corridor_gives_a_two_point_path() { /* from, to -> [from', to'] */ }
#[test] fn a_path_goes_around_a_wall() { /* U-образная карта: путь длиннее прямой и все точки на сетке */ }
#[test] fn an_unreachable_target_gives_none() { /* два острова без связи */ }
#[test] fn a_path_crosses_a_tile_border() { /* две плитки рядом, путь через стык */ }
#[test] fn area_cost_changes_the_route() { /* два пути: короткий через зону 'болото' и длинный; cost(болото)=10 -> выбирает длинный; cost=1 -> короткий */ }
#[test] fn infinite_cost_blocks_the_area() { /* единственный проход через зону с INFINITY -> None */ }
#[test] fn bad_costs_are_rejected() { /* NaN, -1 -> Err(BadCost) */ }
#[test] fn replacing_a_tile_changes_the_answer() { /* удалить плитку-мост -> None; вернуть -> Some */ }
#[test] fn nearest_point_snaps_to_the_mesh_and_respects_the_limit() { /* точка над полом -> на полу; далеко -> None */ }
#[test] fn raycast_stops_at_the_mesh_edge() { /* луч через край карты -> точка на краю */ }
#[test] fn random_points_lie_on_the_mesh() { /* 200 точек, nearest_point(p, 1e-3) == Some(p) */ }
```

## Шаг 2 — off-mesh связи, запросы с лимитом (M)
```rust
pub struct NavLink { pub from: Vec3, pub to: Vec3, pub bidirectional: bool, pub class_mask: u32, pub cost: f32 }
impl NavWorld { pub fn add_link(&mut self, link: NavLink) -> Result<NavLinkId, NavError>; pub fn remove_link(&mut self, id: NavLinkId) -> bool; }
```
`NavLink` → `landmass::AnimationLink` (имя и поля — по `nav-spike.md`). Связь, концы которой не лежат на сетке в пределах `0.5 м`, — `NavError::LinkOffMesh`.
Асинхронные запросы (план 2.2: «на воркерах, кэш, лимит стоимости на кадр»): **упрощение для первой версии**: очередь запросов, обрабатываемая в системе по бюджету *N запросов за кадр* (по умолчанию 16), без потоков.
```rust
pub struct PathQueue { /* pending: VecDeque<PathRequest>, results */ }
impl PathQueue { pub fn submit(&mut self, entity: Entity, from: Vec3, to: Vec3, class: AgentClass) -> PathTicket; pub fn run(&mut self, world: &NavWorld, budget: usize) -> Vec<(PathTicket, Option<Vec<Vec3>>)>; }
```
Кэш по паре полигонов — **не** делать (отложено до профиля).
Тесты: `a_link_lets_the_path_jump_a_gap` (две площадки без общего ребра + связь → путь есть, содержит концы связи); `a_one_way_link_is_not_used_backwards`; `class_mask_hides_the_link_from_other_classes` (**если** spike показал, что landmass поддерживает маски; иначе тест `#[ignore = "landmass has no link filter, see nav-spike.md"]`); `a_link_far_from_the_mesh_is_rejected`;
`the_queue_respects_the_budget` (20 запросов, budget 16 → за первый `run` 16 результатов, за второй 4); `results_keep_submission_order`.

## Шаг 3 — ECS и отладка (S–M)
Компоненты (в `redox_nav`, `components.rs`; ECS — `redox_ecs`, зависимость `redox_nav` → `redox_ecs` допустима, **ПРОВЕРИТЬ** `deny`/слои крейтов по `docs/CRATES.md`):
```rust
pub struct NavAgent { pub class: AgentClass, pub radius: f32, pub height: f32, pub speed: f32, pub accel: f32 }
pub struct NavTarget(pub Vec3);
pub struct NavPath { pub points: Vec<Vec3>, pub next: usize }
// события: PathReached { entity }, PathFailed { entity }
```
Система `nav_path_system(world)`: сущности с `NavAgent`+`NavTarget` без актуального `NavPath` → заявка в `PathQueue`; результат → `NavPath` или событие `PathFailed` и удаление `NavTarget`.
События — как реализован `Events<T>` в `redox_ecs/src/event.rs` (**ПРОВЕРИТЬ** API отправки).
Движение к точкам (простое, без избегания; избегание — NAV-04): `nav_follow_system(world, dt)` двигает `Transform` агента к `points[next]` со скоростью `speed` и ускорением `accel`; достигнув последней точки — событие `PathReached`, удаление `NavTarget`/`NavPath`.
Подключение в `redox_core/src/plugin.rs` как `NavPlugin` (по образцу `AudioPlugin`, `plugin.rs:78–92`): `Stage::Update`, ресурсы `NavWorld`/`PathQueue` вставляются игрой (плагин ничего не грузит сам).
Отладка: `NavDebugDraw` (ресурс, как `AudioDebugDraw` в `redox_audio/src/debug.rs`): `Vec<[Vec3;2]>` линий (рёбра полигонов, пути) для существующего отрисовщика линий (`renderer/debug_draw.rs`; **ПРОВЕРИТЬ** как `AudioDebugDraw` попадает в `redox_ui/src/debug/audio.rs` — повторить).
Хост-API для WASM (`nav_path`, `nav_set_target`) — **вне этой спеки**; отдельная мелкая задача после NAV-04.
Тесты (`tests/ecs.rs`): `an_agent_with_a_target_gets_a_path`; `an_agent_without_a_route_gets_path_failed`; `an_agent_reaches_its_target_and_the_target_is_removed` (карта-коридор, 600 тиков по 1/60 с, `PathReached` отправлен ровно один раз, позиция в радиусе 0,1 от цели); `the_agent_stays_on_the_mesh_while_following` (каждые 10 тиков `nearest_point(pos, 0.05).is_some()`).

## Границы
- **Читать:** `docs/engineering/notes/nav-spike.md`, NAV-02, `redox_nav/src/*`, `redox_core/src/plugin.rs`, `redox_audio/src/{debug,systems}.rs`, `redox_ecs/src/event.rs`.
- **Создать:** `redox_nav/src/{world,backend,links,queue,components,systems,debug}.rs`, `redox_nav/tests/{paths,ecs}.rs`.
- **Менять:** `redox_nav/src/lib.rs`, `redox_core/src/plugin.rs` (+`NavPlugin`), `redox_core/Cargo.toml`, `docs/CRATES.md`.
- **Не трогать:** физику, рендер.

## Проверка
`cargo fmt`, `cargo clippy -p redox_nav -p redox_core --all-targets -- -D warnings`, `cargo test -p redox_nav -p redox_core`, `python scripts/gen_crates_doc.py --check`, `python scripts/check_file_sizes.py`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Типы `landmass` не выходят за `backend.rs`. Любое «в landmass так нельзя» — в отчёт с цитатой исходника, а не обход.
