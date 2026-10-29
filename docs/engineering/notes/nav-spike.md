# NAV-01: спайк навигации — факты из исходников

Статус: каркас крейта `redox_nav` готов, примеры `examples/spike_bake.rs` и `examples/spike_paths.rs` написаны, **не запускались** (компилятора и замеров на этом шаге не было). Таблицу результатов заполняет координатор.

Версии, которые разрешил `cargo fetch` (исходники читались в `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`):

| Крейт | Версия | Примечание |
|---|---|---|
| `rerecast` | 0.4.0 | `#![no_std]`, без `unsafe` (проверено grep) |
| `landmass` | 0.9.3 | без `unsafe` (проверено grep) |
| `dodgy_2d` | 0.5.5 | тянет `glam 0.29`, зависимость `landmass` |
| `glam` | 0.29.3, 0.30.10, 0.32.1, 0.33.12 | четыре версии в дереве; `cargo tree -d` показывает их все |

## Факты из исходников

### Подключение зависимостей
- `rerecast` не имеет фичи `serde`; есть `serialize` (serde-derive). В `redox_nav` `serialize` не включён: формат `.nav` наш. Включены `std` и `libm` (`libm` добавляет `glam/libm`, см. `Cargo.toml` крейта: `libm = ["dep:libm", "glam/libm"]`). Фича `tracing` не нужна.
- `landmass` не имеет обязательных фич; `debug-avoidance` (отладочные данные избегания) не включаем.
- `landmass` реэкспортирует `glam::Vec3` (версия 0.30): `landmass::Vec3`. Прямая зависимость на `glam` не нужна.
- `rerecast` `Vec3A`/`UVec3` и `Aabb3d` строятся через `From<[f32; 3]>` / `Into` без имени типа glam: в `spike_bake.rs` так и сделано.

### rerecast 0.4.0 (запекание)
Публичного «один вызов на плитку» нет. Конвейер собирается из шагов (все в `src/*.rs`):

1. `TriMesh { vertices: Vec<Vec3A>, indices: Vec<UVec3>, area_types: Vec<AreaType> }` (`trimesh.rs`).
   `TriMesh::mark_walkable_triangles(threshold_rad: f32)` ставит `AreaType::DEFAULT_WALKABLE` только треугольникам с нормалью круче порога; остальные оставляет как были.
2. `HeightfieldBuilder { aabb: Aabb3d, cell_size: f32, cell_height: f32 }.build() -> Result<Heightfield, HeightfieldBuilderError>` (`heightfield.rs`). Ширина и высота в ячейках считаются из `aabb`.
3. `Heightfield::populate_from_trimesh(trimesh: TriMesh, walkable_height: u16, walkable_climb: u16) -> Result<(), RasterizationError>`: растеризация и три фильтра (`filter_low_hanging_walkable_obstacles`, `filter_ledge_spans`, `filter_walkable_low_height_spans`). Альтернатива без фильтров: `rasterize_triangles(&TriMesh, walkable_climb)`.
4. `Heightfield::into_compact(self, walkable_height: u16, walkable_climb: u16) -> Result<CompactHeightfield, CompactHeightfieldError>`.
5. `CompactHeightfield::erode_walkable_area(erosion_radius: u16)` (радиус агента в ячейках).
6. `CompactHeightfield::build_distance_field()` (без результата).
7. `CompactHeightfield::build_regions(border_size: u16, min_region_area: u16, merge_region_area: u16) -> Result<(), BuildRegionsError>`.
8. `CompactHeightfield::build_contours(&self, max_error: f32, max_edge_len: u16, flags: BuildContoursFlags) -> ContourSet`.
9. `ContourSet::into_polygon_mesh(self, max_vertices_per_polygon: u16) -> Result<PolygonNavmesh, PolygonNavmeshError>`.
10. `DetailNavmesh::new(mesh: &PolygonNavmesh, heightfield: &CompactHeightfield, sample_distance: f32, sample_max_error: f32) -> Result<DetailNavmesh, DetailNavmeshError>`. Тип ошибки — пустой enum, то есть ошибок нет.

Выход шага 9, `PolygonNavmesh`:
- `vertices: Vec<U16Vec3>` в ячейках; мировая координата = `aabb.min + v * (cell_size, cell_height, cell_size)` (пример в документации `poly_mesh.rs`).
- `polygons: Vec<u16>` фиксированной длиной `max_vertices_per_polygon` на полигон; конец списка — `PolygonNavmesh::NO_INDEX = 0xffff`. Полигоны выпуклые, против часовой стрелки на плоскости XZ (документация).
- `areas: Vec<AreaType>` на полигон; `polygon_count()`.
- `Debug`, `PartialEq`, `Clone` выведены (`#[derive]` на строке 77 `poly_mesh.rs`), определённость проверяется сравнением `format!("{:?}")`.

`DetailNavmesh`: `meshes: Vec<SubMesh>`, `vertices: Vec<Vec3>` (мировые), `triangles: Vec<[u8; 3]>`, `triangle_flags`. Детальная сетка даёт треугольники, не выпуклые полигоны, поэтому в `NavTile` пока не используется (см. расхождения).

Конфигурация: `ConfigBuilder { agent_radius, agent_height, walkable_climb, walkable_slope_angle, cell_size_fraction, cell_height_fraction, ..Default::default() }.build() -> Config`. Формулы из `config.rs`:
- `cell_size = agent_radius / cell_size_fraction`; `cell_height = agent_radius / cell_height_fraction`.
- `walkable_radius = ceil(agent_radius / cell_size)`; `border_size = walkable_radius + 3`.
- `walkable_height = ceil(agent_height / cell_height)`; `walkable_climb = floor(walkable_climb / cell_height)`.
- `max_edge_len = walkable_radius * edge_max_len_factor`; `min_region_area = min_region_size^2`; `merge_region_area = merge_region_size^2`.
- Для радиуса 0.4 и ячейки 0.2: `cell_size_fraction = 2.0`, `cell_height_fraction = 4.0` дают `cell_height = 0.1`. Значит `walkable_radius = 2`, `walkable_height = 18`, `walkable_climb = 4`. Совпадает с ожидаемыми значениями NAV-02.
- `Config.tile_size`, `tiling` существуют, но при `tiling = true` библиотека не строит плитки за нас: `width/height` берутся как `tile_size + 2*border`. Драйвер плиток (вырезка геометрии с границей, сборка каждой плитки отдельно) — наш.

Определённость: в крейте нет `unsafe`, нет глобального состояния (проверено чтением `lib.rs`); структуры сортируются `SlotMap` без рандома. Побитовое совпадение между запусками и потоками нужно проверить замером.

Области: `AreaType(pub u8)`, `NOT_WALKABLE = 0`, `DEFAULT_WALKABLE = 255`; `is_walkable()` — любое значение кроме 0. См. расхождение 3.

### landmass 0.9.3 (пути и избегание)
- Система координат `XYZ`: в комментарии `coords.rs` написано «X вправо, Y вперёд, Z вверх», поворот `Transform` — вокруг оси Z. То есть landmass — Z-вверх, rerecast и движок — Y-вверх. В спайке используется своя `YUp` (`coords.rs` трейт `CoordinateSystem`): `to_landmass(v) = (x, z, y)`, `FLIP_POLYGONS = true`, потому что перестановка Y и Z зеркалит плоскость и переворачивает обход полигонов.
- `Archipelago<CS>`: `new(ArchipelagoOptions)`, `add_island(Island) -> IslandId`, `remove_island(id)` (паникует, если id нет: `expect`), `get_island_mut(id) -> Option<IslandMut>` (deref к `Island`, `set_nav_mesh`, `set_transform`), `add_agent(Agent) -> AgentId`, `remove_agent` (тоже паникует), `get_agent(_mut)`, `add_animation_link(AnimationLink) -> AnimationLinkId`, `remove_animation_link` (паника при отсутствии), `set_type_index_cost(usize, f32) -> Result<(), SetTypeIndexCostError>`, `update(dt)`.
- `ArchipelagoOptions<CS>`: `point_sample_distance`, `neighbourhood`, `avoidance_time_horizon`, `obstacle_avoidance_time_horizon`, `reached_destination_avoidance_responsibility`. `from_agent_radius(r)` задаёт `neighbourhood = 10r`, горизонты 0.5 и 0.25, responsibility 0.1; `PointSampleDistance3d` — горизонтально 0.2r, вверх 0.5r, вниз r, предпочтение вертикали 2.0, для связей 0.5r.
- `NavigationMesh<CS> { vertices, polygons: Vec<Vec<usize>>, polygon_type_indices: Vec<usize>, height_mesh: Option<..> }.validate() -> Result<ValidNavigationMesh<CS>, ValidationError>`. Проверки: длины, минимум 3 вершины, индексы, вырожденные рёбра, ребро больше чем у двух полигонов (`DoublyConnectedEdge`), вогнутость (`ConcavePolygon`), порядок обхода.
- `Island::new(Transform<CS>, Arc<ValidNavigationMesh<CS>>)`; `Transform { translation, rotation }` (вращение вокруг Z).
- `Archipelago::update(dt)`: перестраивает связи для грязных островов (`edge_link_distance = 0.01`, жёстко, в коде помечено TODO), сэмплирует агентов, ищет пути для тех, у кого их нет или они устарели, считает избегание и пишет `desired velocity`. Позиции агент не интегрирует: это делает игра.
- `sample_point(point, &PointSampleDistance) -> Result<SampledPoint, SamplePointError>`; пока данные грязные (после изменения острова без `update`), возвращает `NavDataDirty`. `SampledPoint` держит заимствование архипелага, поэтому после `update` (нужен `&mut`) точки нужно сэмплировать заново.
- `find_path(&SampledPoint, &SampledPoint, &HashMap<usize, f32>, PermittedAnimationLinks) -> Result<Vec<PathStep<CS>>, FindPathError>`. `FindPathError::{NonPositiveTypeIndexCost, NoPathFound}`. `PathStep::{Waypoint, AnimationLink { start_point, end_point, link_id }}`. Внутри `assert!`, что данные не грязные.
- `Agent::create(position, velocity, radius, desired_speed, max_speed)`. Поля публичные: `current_target: Option<Coordinate>`, `target_reached_condition`, `permitted_animation_links`, `paused`, `override_type_index_to_cost`. `get_desired_velocity() -> &Coordinate`, `state() -> AgentState` (Idle, ReachedTarget, Moving, NoPath, AgentNotOnNavMesh, TargetNotOnNavMesh, Paused и другие).
- `AnimationLink<CS> { start_edge: (C, C), end_edge: (C, C), kind: usize, cost: f32, bidirectional: bool }`. Связь задаётся ребром, а не точкой.
- `PermittedAnimationLinks::{All, Kinds(Arc<HashSet<usize>>)}` — фильтр по `kind`, задаётся агенту и `find_path`.
- Ограничения по `unsafe`: не найдены.

## Расхождения со спекой

1. **Фича `serde` у rerecast не существует**, нужна `serialize`. Решено: `std` + `libm`, без сериализации (формат `.nav` наш).
2. **Система координат landmass Z-вверх**, движок Y-вверх. Нужна своя `CoordinateSystem` с переставленными осями и `FLIP_POLYGONS = true`. Типы landmass не выходят за `backend.rs`, поэтому это внутреннее дело крейта.
3. **Инверсия идентификаторов областей.** В спеке `NavAreaId::WALKABLE = 0`, `NOT_WALKABLE = 255`. В rerecast `NOT_WALKABLE = 0`, а любое ненулевое значение проходимо. Предложение для NAV-02: в `bake.rs` кодировать `AreaType(area.0 + 1)` для `area.0 < 255` и декодировать обратно; `NOT_WALKABLE` в растеризатор не подавать вовсе.
4. **Нет API плиток.** Вырезка с границей, сборка по плиткам, проверка совпадения рёбер на стыках — наша работа в `bake.rs`. Практическая неясность: `border_size` в rerecast обрезает область у края AABB, и как точно совпадут полигоны соседних плиток, покажет замер (spike A, проверка «плитки»).
5. **Детальная сетка не даёт выпуклых полигонов.** Спека NAV-02 требует выпуклые полигоны; в `NavTile` берём `PolygonNavmesh`. Высоты стыков и лестниц могут быть грубее детальной сетки. Если тест лестницы не пройдёт, понадобится учёт `DetailNavmesh`.
6. **Параметры избегания не per-agent.** В landmass `neighbourhood`, горизонты избегания и `responsibility` заданы на весь `Archipelago` (`ArchipelagoOptions`). Поля `max_neighbours` и `neighbour_distance` per-agent в `AvoidanceParams` (NAV-04) не поддерживаются напрямую. Ограничения: один набор параметров на архипелаг; `max_neighbours` не имеет аналога.
7. **Фильтра связей по маске класса нет.** Есть `kind: usize` и `PermittedAnimationLinks::Kinds` у агента/запроса. Для `class_mask` в NAV-03 предлагается `kind = class_mask as usize`, а набор разрешённых видов собирать из битов класса. Тест `class_mask_hides_the_link_from_other_classes` поэтому реализуем, а не помечаем игнорируемым.
8. **Паники на отсутствующих id**: `remove_island`, `remove_agent`, `remove_animation_link`, `remove_character` вызывают `expect`. Фасад `NavWorld` обязан проверять наличие (`get_*`) до удаления.
9. **Допуск соединения островов жёстко 0.01 м** (`nav_data.rs`, `edge_link_distance = 0.01`). Стыки плиток запекаются по общим вершинам с точностью 1e-3, это укладывается; при неточности стыка путь не найдётся.
10. **Мультирадиус.** Агенты разного радиуса в одном `Archipelago` добавляются (радиус — поле агента). Но навмеш запечён под один радиус: в одном архипелаге разные классы агентов не имеют отдельных сеток. Это совпадает с допущением «один класс» в NAV-01.

## Результаты (заполняет координатор)

| Проверка | Критерий «годится» | Факт |
|---|---|---|
| rerecast: комната/лестница | покрытие пола, проём проходим, лестница одна поверхность | |
| rerecast: плитки | общие рёбра сходятся, нет щелей | |
| rerecast: определённость | побитно одинаково, в т.ч. из другого потока | |
| rerecast: время (c), release | ≤ 500 мс на всю сцену | |
| landmass: переход между плитками | путь найден и идёт через стык | |
| landmass: замена острова | агент не телепортируется | |
| landmass: off-mesh | путь использует связь | |
| landmass: 1000 запросов | p99 ≤ 200 мкс | |
| landmass: 200 агентов | ≤ 1 мс/кадр, 0 застрявших | |
| landmass: два радиуса | да / нет (если нет — один класс) | |
