# NAV-04 — Навигация: избегание столкновений (ORCA через `landmass`/`dodgy_2d`), движение через `CharacterController`, динамические препятствия

- **Размер:** M + M + S. **Крейт:** `redox_nav`, `redox_core` (подключение). **Зависит от:** NAV-03; для шага 2 — `CharacterController` (PHY-01…04 готовы, `redox_physics/src/character.rs`). **Блокирует:** ничего.
- **Модель:** haiku.
- Допущение владельца: цель — 200 агентов < 1 мс на **одном ядре**, release (NAV-01).

## Суть
Агенты NAV-03 идут по маршруту, но проходят друг сквозь друга и сквозь физические препятствия. Нужно: (1) локальное избегание толпы, (2) движение через физику, (3) временные запреты на проходимость для динамических препятствий.

## Шаг 1 — избегание (M)
Используем встроенное избегание `landmass` (внутри `dodgy_2d`), включаемое/настраиваемое через `backend.rs`. Фасад:
```rust
/// Per-agent avoidance settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AvoidanceParams { pub neighbour_distance: f32, pub max_neighbours: u32, pub time_horizon: f32, pub obstacle_time_horizon: f32 }
impl Default for AvoidanceParams { /* 5.0 м, 10, 2.0 с, 1.0 с — стартовые значения dodgy_2d; уточнить по nav-spike.md */ }

impl NavWorld {
    /// Registers an agent for avoidance. The returned id is used by `update_agents`.
    pub fn add_agent(&mut self, position: Vec3, radius: f32, max_speed: f32, params: AvoidanceParams) -> Result<NavAgentId, NavError>;
    pub fn remove_agent(&mut self, id: NavAgentId) -> bool;
    /// Sets where the agent wants to go (None = stand still).
    pub fn set_goal(&mut self, id: NavAgentId, goal: Option<Vec3>);
    /// Feeds the current positions and velocities, advances the avoidance by `dt` and returns the desired velocity of each agent (world space, on the mesh plane).
    pub fn update_agents(&mut self, dt: f32, poses: &[(NavAgentId, Vec3, Vec3)]) -> Vec<(NavAgentId, Vec3)>;
}
```
`nav_follow_system` (NAV-03) заменяется (при наличии `NavWorld` с агентами) на `nav_avoid_system`: собирает позы агентов, вызывает `update_agents`, записывает желаемую скорость в компонент `NavDesiredVelocity(Vec3)`; интеграция позиции — шаг 2. Старая система остаётся для случая без избегания.
Тесты (`tests/avoidance.rs`; карта — широкий прямоугольный пол, агенты шагают по 1/60 с, позиции интегрируются в самом тесте: `p += v*dt`):
```rust
#[test] fn two_agents_head_on_pass_each_other() { /* стартуют на расстоянии 10 м, цели друг у друга; минимальное расстояние между центрами за всё время >= 0.9 * (r1+r2); оба достигают цели за 15 с */ }
#[test] fn a_lone_agent_walks_straight_to_its_goal() { /* путь не длиннее прямой * 1.05 */ }
#[test] fn an_agent_without_a_goal_stays_put() { /* скорость ~0 */ }
#[test] fn a_crowd_of_two_hundred_reaches_the_goal_without_getting_stuck() { /* 200 агентов в сетке 20x10 идут через проём 3 м; за 60 с >= 195 дошли; ни одна пара не пересекается глубже 50% суммы радиусов */ }
#[test] fn removing_an_agent_forgets_it() { /* remove_agent -> true; second -> false; update_agents не возвращает его */ }
```
Бенчмарк (не тест): `benches/avoidance.rs` (criterion, **ПРОВЕРИТЬ** что criterion уже в workspace: `grep -rn criterion Cargo.toml crates/*/Cargo.toml`; иначе замер `std::time::Instant` в `#[test] #[ignore]` с печатью) — 200 агентов, среднее время `update_agents`; цель ≤ 1 мс. Результат записать в ADR 0007 («Measured»). Если > 1 мс — **не оптимизировать**, отчитаться (запасной план в ADR: параллельный обход агентов).

## Шаг 2 — движение через физику (M)
Система `nav_move_system(world, dt)`: для сущностей с `NavAgent`, `NavDesiredVelocity` и `CharacterController` (`redox_physics::components`) выставляет желаемое смещение `desired_velocity * dt` (поле/метод — **ПРОВЕРИТЬ** в `character.rs` и `components.rs`: после PHY-01 контроллер получает `translation` как желаемое перемещение; гравитация и прыжок остаются у контроллера). Реальную скорость (после коллизий) читать обратно из разности позиций за тик и передавать в `update_agents` как текущую скорость.
Агенты без `CharacterController` двигаются напрямую по `Transform` (как в NAV-03).
Привязка к сетке: после перемещения, если `nearest_point(pos, 0.5)` есть — мягко подтягивать Y к сетке (нужно для лестниц; коэффициент 0,5 за тик); если сетки рядом нет (оттолкнули физикой) — агент остаётся на месте и пересчитывает путь (`PathFailed` не слать, это временная ситуация).
Тесты (`redox_core/tests` или `redox_nav/tests/physics.rs` с `PhysicsContext`, как `redox_audio/tests/occlusion.rs`):
```rust
#[test] fn an_agent_stops_at_a_wall_the_mesh_does_not_know_about() { /* на сетке сплошной пол, но в физике поставлена стена на пути; агент не проходит сквозь неё за 5 с (z не пересекает стену) */ }
#[test] fn an_agent_with_a_controller_reaches_a_goal_in_the_open() { /* коридор без препятствий: достиг цели в пределах 0.3 м */ }
#[test] fn the_agent_follows_a_ramp_in_height() { /* пол с подъёмом на 1 м на 4 м пути: Y конца ≈ высота цели ± 0.2 */ }
```
(Если стыковка контроллера и сетки в тесте требует `step()` физики на каждом тике — как в `tests/character_v2.rs`; там образец.)

## Шаг 3 — динамические препятствия (S)
Вырезание из плиток (carving) — **не делаем** (ADR: «real carving is deferred»). Делаем «временный запрет»:
```rust
impl NavWorld {
    /// Marks the convex area (given as a polygon footprint on the XZ plane, with a height range) not walkable until removed. Returns an id.
    pub fn add_blocker(&mut self, footprint: &[Vec3], min_y: f32, max_y: f32) -> Result<NavBlockerId, NavError>;
    pub fn remove_blocker(&mut self, id: NavBlockerId) -> bool;
}
```
Реализация первой версии — **перезапекание затронутой плитки** с дополнительными треугольниками `NOT_WALKABLE` (нужен хранимый `NavGeometry` плитки → ограничение: работает только для миров, созданных из геометрии, не из `.nav`; для `.nav`-миров `add_blocker` возвращает `NavError::NoSourceGeometry`). Альтернатива, если перезапекание дороже 50 мс на плитку (замер в NAV-01 Spike A): запретить полигоны, пересекающиеся с футпринтом, по их центрам (грубо, но без перезапекания) — выбрать это как реализацию по умолчанию, перезапекание оставить на потом.
Тесты: `a_blocker_cuts_the_route` (коридор 2 м, блокер на всю ширину → `find_path` = None; снят → Some); `a_blocker_on_a_wide_floor_makes_a_detour` (путь длиннее прямой, не заходит в футпринт); `removing_an_unknown_blocker_returns_false`.

## Границы
- **Читать:** NAV-03, `nav-spike.md`, `redox_physics/src/{character.rs,components.rs}`, `redox_physics/tests/character_v2.rs`, `redox_audio/tests/occlusion.rs` (как строится мир с физикой в тесте).
- **Создать:** `redox_nav/src/{avoid,blockers}.rs`, `redox_nav/tests/{avoidance,physics,blockers}.rs`, `redox_nav/benches/avoidance.rs`.
- **Менять:** `redox_nav/src/{world,systems,components,lib}.rs`, `redox_core/src/plugin.rs` (если нужен новый порядок систем), `redox_nav/Cargo.toml` (+ `redox_physics`; dev-dep `rapier3d` для тестов, как в `redox_audio`).
- **Не трогать:** внутренности физики.

## Проверка
`cargo fmt`, `cargo clippy -p redox_nav -p redox_core --all-targets -- -D warnings`, `cargo test -p redox_nav`, `cargo test --release -p redox_nav -- --ignored avoidance_bench --nocapture` (число в отчёт), `python scripts/check_file_sizes.py`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Если `landmass` не поддерживает нужное (маска связей, радиус на агента) — зафиксировать в отчёте, не обходить.
