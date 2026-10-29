# NAV-02 — Навигация: запекание navmesh по плиткам, формат `.nav`, команда `redox_cook bake-navmesh`

- **Размер:** M + M (шаги 1–2) + S (шаг 3). **Крейты:** `redox_nav`, `redox_cook`. **Зависит от:** NAV-01 (**таблица результатов должна быть «годится» для rerecast**; сигнатуры API — из `docs/engineering/notes/nav-spike.md`). **Блокирует:** NAV-03.
- **Модель:** haiku. Допущения по ответам владельца — как в NAV-01 (один класс агента, только заранее запечённые плитки).
- Если spike вернул «rerecast не годится» — спека не выполняется, координатор пишет её заново под запасной план ADR.

## Суть
Превратить геометрию уровня в набор плиток навмеша, сохранить их в собственном формате `.nav` и запекать командой `redox_cook bake-navmesh`.
Публичные типы — свои (`NavTile`, `NavAreaId`, `AgentParams`), `rerecast` не виден снаружи крейта.

## Типы (шаг 1: чистые данные и формат, S, без rerecast)
```rust
/// Parameters of the agent the mesh is built for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AgentParams { pub radius: f32, pub height: f32, pub max_step: f32, pub max_slope_deg: f32 }
impl Default for AgentParams { /* 0.4, 1.8, 0.4, 45.0 */ }

/// Area type of a polygon; costs are set when the world is built (NAV-03).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NavAreaId(pub u8);
impl NavAreaId { pub const WALKABLE: Self = Self(0); pub const NOT_WALKABLE: Self = Self(255); }

/// One baked tile: convex polygons over shared vertices, in world space.
#[derive(Clone, Debug, PartialEq)]
pub struct NavTile {
    pub coord: (i32, i32),            // tile grid coordinate
    pub vertices: Vec<[f32; 3]>,
    pub polygons: Vec<NavPolygon>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct NavPolygon { pub indices: Vec<u16>, pub area: NavAreaId }   // 3..=6 indices, counter-clockwise seen from above (+Y)

/// All tiles of one level for one agent class.
#[derive(Clone, Debug, PartialEq)]
pub struct NavData { pub agent: AgentParams, pub tile_size: f32, pub tiles: Vec<NavTile> }

impl NavData {
    /// Serialises to the `.nav` format. Deterministic: tiles sorted by `coord`.
    pub fn to_bytes(&self) -> Vec<u8>;
    /// Parses `.nav` bytes. Errors: bad magic, unknown version, truncated data, index out of range, polygon with < 3 or > 6 indices.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, NavFormatError>;
}
```
Формат `.nav` (little endian): магия `b"RNAV"`, `u32 version = 1`, `AgentParams` (4×f32), `tile_size f32`, `u32 tile_count`, на плитку: `i32 x, i32 y, u32 vcount, u32 pcount`, вершины `3×f32`,
на полигон `u8 area, u8 n, n × u16`. Ошибка — `thiserror`-перечисление `NavFormatError` (как `MusicError` в `redox_audio`).
Тесты (юнит):
```rust
#[test] fn bytes_roundtrip_is_lossless() { /* NavData with 2 tiles -> to_bytes -> from_bytes == original */ }
#[test] fn serialisation_is_deterministic_and_sorted() { /* tiles given in reverse order produce the same bytes */ }
#[test] fn truncated_data_is_an_error() { /* every prefix of a valid file (except full) is Err, no panic */ }
#[test] fn a_wrong_magic_or_version_is_rejected() { /* ... */ }
#[test] fn an_index_out_of_range_is_rejected() { /* polygon index >= vertex count */ }
#[test] fn a_polygon_with_two_indices_is_rejected() { /* ... */ }
```

## Шаг 2 — запекание (M)
```rust
/// Triangle soup with an area per triangle, in world space.
pub struct NavGeometry { pub vertices: Vec<[f32; 3]>, pub triangles: Vec<[u32; 3]>, pub areas: Vec<NavAreaId> }   // areas.len() == triangles.len()

/// Bakes `geometry` into tiles of `tile_size` metres for `agent`.
pub fn bake(geometry: &NavGeometry, agent: &AgentParams, tile_size: f32) -> Result<NavData, BakeError>;
```
Тело: разбить пространство на плитки по сетке, для каждой плитки выбрать треугольники, пересекающие плитку с полем `border`, вызвать `rerecast` по API, зафиксированному в `nav-spike.md`, собрать `NavTile`.
Параметры rerecast (ячейка `cell_size = agent.radius / 2`, `cell_height = cell_size / 2`, `walkable_climb`, `walkable_slope`, `walkable_radius`, `walkable_height` в ячейках) — перевод метры→ячейки отдельной чистой функцией
`fn voxel_config(agent: &AgentParams) -> VoxelConfig` (**наш** тип с полями в ячейках, юнит-тесты без rerecast):
```rust
#[test] fn the_default_agent_gives_known_voxel_counts() { /* radius 0.4, cell 0.2 -> walkable_radius 2; height 1.8/0.1 -> 18; step 0.4/0.1 -> 4 */ }
#[test] fn a_tiny_agent_never_gets_zero_cells() { /* radius 0.01 -> walkable_radius >= 1 */ }
```
Ошибки: `BakeError::{EmptyGeometry, BadTileSize, Backend(String)}`; пустая плитка (нет полигонов) в `NavData` не пишется.
Тесты (по сценам из NAV-01 — копия геометрии в `tests/bake.rs`):
```rust
#[test] fn a_room_with_a_doorway_is_covered_and_connected() { /* полигоны покрывают >= 90% пола 10x10 за вычетом отступа радиуса; ни одна вершина не внутри стены; проём проходим: точка слева и справа достижимы по рёбрам полигонов (BFS по общим рёбрам внутри теста) */ }
#[test] fn stairs_within_the_step_height_stay_walkable() { /* 8 ступеней по 0.15 -> верхняя площадка достижима BFS */ }
#[test] fn an_obstacle_splits_the_coverage() { /* высокая стена поперёк коридора -> BFS между сторонами не находит пути */ }
#[test] fn a_not_walkable_area_is_excluded() { /* области NOT_WALKABLE не дают полигонов */ }
#[test] fn baking_twice_is_bit_identical() { /* to_bytes одинаковы; и из std::thread::spawn */ }
#[test] fn neighbouring_tiles_share_their_border_edges() { /* пол 40x40, tile 16: рёбра на границе плиток сходятся */ }
#[test] fn empty_geometry_is_an_error() { /* ... */ }
```

## Шаг 3 — геометрия сцены и команда (S)
- `redox_nav::geometry`: `fn from_triangle_meshes(meshes: impl Iterator<Item = (&[[f32;3]] positions, &[u32] indices, Mat4 transform, NavAreaId)>) -> NavGeometry` — чистая функция (трансформирует вершины в мир; сворачивание порядка обхода — тест).
  Источник мешей в игре: **ПРОВЕРИТЬ** `redox_scene` (статические меши сцены) и `redox_physics` (коллайдеры → `to_trimesh` из `parry`); для `redox_cook` достаточно мешей сцены/glTF. Не добавлять зависимость `redox_nav` → `redox_render`.
- `redox_cook bake-navmesh <scene-or-gltf-input> <out.nav> [--tile 16] [--radius 0.4] [--height 1.8] [--step 0.4] [--slope 45]`: разбор строки команд — по образцу `Command::Texture` (`redox_cook/src/main.rs:~30`), с тестом разбора (`parse` без запуска); чтение входа — glTF/GLB (крейт `gltf` уже в `redox_anim`; **ПРОВЕРИТЬ** зависимости `redox_cook`); обновить `USAGE` и шапку модуля.
- Тесты: разбор команды (`bake-navmesh a.glb out.nav` → `Command::BakeNavmesh {…}`; неизвестный флаг — ошибка); `tests/cook_nav.rs`: запечь встроенный в тест маленький GLB (пол + стена, собрать в памяти как тест-ассет в `redox_anim`) и проверить, что `NavData::from_bytes(out)` читается и не пуст.

## Границы
- **Читать:** `docs/engineering/notes/nav-spike.md`, `docs/adr/0007-navigation.md`, `redox_cook/src/main.rs`, `redox_cook/src/bake.rs`, `redox_audio/src/music_play.rs` (образец `thiserror`-ошибок).
- **Создать:** `redox_nav/src/{data,format,voxel,bake,geometry,error}.rs`, `redox_nav/tests/bake.rs`, `redox_cook/tests/cook_nav.rs` (если есть каталог тестов; иначе юниты).
- **Менять:** `redox_nav/src/lib.rs`, `redox_cook/src/main.rs`, `redox_cook/Cargo.toml` (+ `redox_nav`), `docs/CRATES.md`.
- **Не трогать:** `redox_physics`, `redox_scene`, рендер.

## Проверка
`cargo fmt`, `cargo clippy -p redox_nav -p redox_cook --all-targets -- -D warnings`, `cargo test -p redox_nav -p redox_cook`, `cargo deny check`, `python scripts/gen_crates_doc.py --check`, `python scripts/check_file_sizes.py`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Типы `rerecast` не выходят за `bake.rs`/`voxel.rs`. Не вводить рантайм-запекание (NAV-03 только читает готовые плитки).
