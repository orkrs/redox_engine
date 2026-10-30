# PHY-07 — Ragdoll: из скелета в тела и шарниры, переход анимация ↔ физика

- **Статус:** шаги 1–4 выполнены 2026-10-29 (код Haiku, проверен сборкой и тестами); шаг 5 выполнен 2026-10-30: оконный пример `ragdoll_window` (см. «Исход шага 5» ниже). Допущения: ось колена X (поза Mixamo), капсула персонажа при входе в ragdoll не удаляется. `redox_physics` теперь зависит от `redox_anim`.

- **Размер:** 5 шагов (S, M, M, M, S) ≈ 9 дней. **Крейты:** `redox_physics`, `redox_anim` (чтение скелета), `redox_core` (система). **Зависит от:** PHY-04 (компонент `Joint`), PHY-05. **Блокирует:** —.
- **Модель:** haiku по шагам 1, 2, 5; шаги 3–4 — haiku с проверкой координатора (физическая настройка — сложное место: стратегия «останови и сообщи»).
- Объём — «минимум»: человекоподобный ragdoll из шаблона с ограничениями углов, включение по событию (смерть, удар), плавный возврат в анимацию **не** входит (только «упал → лежит» и «переключить обратно рывком»). **Не входит:** частичный ragdoll (только руки), активный ragdoll (мышцы под управлением анимации), разрушение костей.

## Суть и аудит
- Шарниры есть (B4 ✅): `JointKind::{Fixed, Hinge, Slider, Ball, Spring, Rope, Generic}`, лимиты `limits: Option<(f32,f32)>` **только для Hinge/Slider**, моторы, разрыв; компонент ECS `Joint` + `JointLink` (`sync_joints`). Тела: `PhysicsContext::add_rigid_body(entity, …)`/`remove_rigid_body`, формы `ColliderShape`.
- Скелет: `redox_anim::Skeleton { bones: Vec<Bone { name, parent, bind_pose, inv_bind }> }`, `Animator`/`GraphPlayer` выдают позы костей (матрицы моделей), деформация `SkinnedGeometry::deform_into` берёт матрицы кожи.
- Нет: ограничения конуса/скручивания для `Ball` (для плеча/бедра необходимо), шаблона ragdoll, связи «кость ↔ тело», переключателя режима, коллизионных слоёв «кость не бьёт свою кость».

## Шаг 1 — лимиты для `Ball`: конус и скручивание (M, `redox_physics`)
- `JointKind::Ball` получает поля через новый вариант `BallLimited { swing_limit: f32 /*рад, конус*/, twist_min: f32, twist_max: f32 }` (старый `Ball` остаётся без лимитов; изменений в существующих вариантах нет → обратная совместимость).
- Реализация поверх rapier: сферический шарнир + лимиты через `SphericalJointBuilder::limits(JointAxis::AngX/AngY/AngZ, [min, max])` (**ПРОВЕРИТЬ** API rapier 0.36: `GenericJoint::set_limits`; swing — AngY/AngZ симметрично `±swing_limit`, twist — AngX). Конус аппроксимируется квадратом ±swing по двум осям (допущение, записать в doc-комментарий).
- Тесты (`crates/redox_physics/tests/joint_limits.rs`, рядом с существующими):
```rust
#[test] fn a_limited_ball_cannot_swing_beyond_its_cone() { /* маятник на BallLimited(swing 45°) с боковой силой: угол отклонения ≤ 45°+2° после 3 с */ }
#[test] fn the_twist_is_limited() { /* закрутка моментом: угол вокруг оси в [min,max] ±2° */ }
#[test] fn an_unlimited_ball_still_swings_freely() { /* регресс: старое поведение, угол > 90° */ }
#[test] fn the_limits_hold_while_the_joint_is_stressed_but_not_broken() { /* break_force большая: шарнир цел */ }
```

## Шаг 2 — шаблон ragdoll и данные (S)
- `redox_physics/src/ragdoll.rs`:
```rust
/// One body of a ragdoll template.
pub struct RagdollBone { pub bone: String, pub parent: Option<usize> /*индекс в шаблоне*/, pub shape: RagdollShape /*Capsule{radius, half_height}|Box|Sphere*/, pub mass: f32, pub joint: Option<RagdollJoint>, pub offset: Vec3 /*центр тела от кости, в локальных осях кости*/ }
pub struct RagdollJoint { pub kind: RagdollJointKind /*Hinge{axis, min, max}|Ball{swing, twist_min, twist_max}*/, pub anchor: Vec3 }
pub struct RagdollTemplate { pub name: String, pub bones: Vec<RagdollBone> }
impl RagdollTemplate {
    pub fn parse_ron(text: &str) -> Result<Self, RagdollError>;
    pub fn validate(&self, skeleton: &Skeleton) -> Result<(), RagdollError>;   // все кости есть, родитель раньше ребёнка, одна корневая, массы > 0
    pub fn humanoid(skeleton: &Skeleton) -> Result<Self, RagdollError>;       // автошаблон по именам костей: pelvis/hips, spine*, head, upperarm*/forearm*/hand*, thigh*/calf*/foot* (case-insensitive, суффиксы .L/.R/_l/_r/Left/Right)
}
```
- `humanoid` берёт размеры из скелета (длина кости = расстояние до дочерней, радиус = 0,12 × длина, не меньше 3 см), массы по таблице (таз 10 кг, бедро 6, голень 3, и т. д. — константы в модуле), сочленения: колено/локоть — Hinge ±(0..2,4 рад), плечо/бедро/позвоночник/шея — BallLimited (swing 60°/70°/20°/35°, twist ±25°…).
Тесты (юнит):
```rust
#[test] fn the_humanoid_template_covers_a_standard_skeleton() { /* скелет из 15 костей Mixamo-стиля -> 15 тел, у корня нет шарнира */ }
#[test] fn a_missing_bone_is_reported_with_its_name() { /* ... */ }
#[test] fn parents_precede_children() { /* ... */ }
#[test] fn the_template_roundtrips_through_ron() { /* ... */ }
#[test] fn bone_names_with_side_suffixes_are_recognised() { /* upperarm.L / UpperArm_Right / LeftUpperArm */ }
#[test] fn masses_sum_to_a_plausible_body_mass() { /* 55–100 кг */ }
```

## Шаг 3 — создание и снятие ragdoll (M, `redox_physics`)
- `PhysicsContext::spawn_ragdoll(world: &mut World, template: &RagdollTemplate, bone_poses: &[Mat4] /*мировые позы костей*/, root_velocity: Vec3, group: u32) -> Result<RagdollHandle, PhysicsError>`: на каждую кость создаёт сущность-тело (`add_rigid_body` динамика, коллайдер по форме, смещённый на `offset`, позы из `bone_poses`), соединяет шарнирами по шаблону (`add_joint`, якоря в локальных осях тел), назначает **один коллизионный слой/группу** для всех костей и отключает контакты между соседями по цепи (**ПРОВЕРИТЬ** слои: `layer_config.rs`, `ContactFilter`; если фильтра пар нет — отключить через `collision_groups` rapier `InteractionGroups` по индексу).
- Начальная скорость: линейная скорость корня передаётся всем телам.
- `remove_ragdoll(handle)` — удалить тела и шарниры.
- Компонент ECS `RagdollBody { ragdoll: RagdollHandle, bone_index: usize }`.
Тесты (`tests/ragdoll.rs`):
```rust
#[test] fn a_spawned_ragdoll_falls_and_lands_on_the_floor() { /* humanoid над плоскостью: через 3 с центр тела на высоте ≤ 0.4 м, скорость < 0.2 */ }
#[test] fn the_joints_keep_the_body_connected() { /* расстояния между якорями соседних костей ≤ 3 см */ }
#[test] fn adjacent_bones_do_not_collide_with_each_other() { /* стек сидит без «взрыва»: макс скорость тела < 20 м/с за 2 с */ }
#[test] fn the_knee_does_not_bend_backwards() { /* лимит колена: угол в [0, 2.4] ±3° после падения на колени */ }
#[test] fn the_initial_velocity_is_inherited() { /* корень 5 м/с вперёд -> через 0.2 с все тела сместились вперёд ≥ 0.5 м */ }
#[test] fn removing_a_ragdoll_removes_all_bodies_and_joints() { /* счётчики rapier */ }
#[test] fn two_ragdolls_do_not_interact_through_their_own_layer_but_collide_with_the_world() { /* ... */ }
```

## Шаг 4 — связь со скиннингом и режим «анимация ↔ ragdoll» (M, `redox_core` / `redox_anim`)
- Система `ragdoll_pose_system(world)`: для включённых ragdoll'ов читает позы тел (`RagdollBody`) и пересчитывает матрицы костей: `bone_world = body_world * offset⁻¹`, затем локальные позы относительно родителя, **передаёт в скиннинг** (`SkinnedMeshBinding::set_pose(&[Mat4])` — **ПРОВЕРИТЬ** как сейчас передаются позы из `GraphPlayer::update`; добавить вход «внешние мировые позы», не меняя путь анимации).
- Переключатель `CharacterPoseMode::{Animated, Ragdoll}` (компонент): `Animated → Ragdoll` вызывает `spawn_ragdoll` с текущими позами костей (кадр без скачка) и скоростью контроллера; `Ragdoll → Animated` — `remove_ragdoll` и сброс анимации на позу тела (в этой спеке — рывком, без блендинга; блендинг — отдельная задача).
- Контроллер персонажа в режиме Ragdoll отключается (капсула удаляется/не двигается).
Тесты (без GPU, `redox_core/tests/ragdoll_pose.rs`): `bone_poses_follow_the_bodies` (после падения позы костей = позы тел ± 1e-4), `switching_to_ragdoll_keeps_the_pose_in_the_first_frame` (разность матриц костей до/после < 1e-3), `switching_back_restores_the_animator_output`, `the_controller_is_inactive_while_ragdolled`, `a_skinned_mesh_receives_the_ragdoll_pose` (деформация вершины рядом с головой смещается вместе с телом головы).

## Шаг 5 — пример и документация (S)
Пример `crates/redox_core/examples/ragdoll_demo.rs`: лиса из Foxfire (или куб-человек, если glTF неудобен) падает при нажатии `R`, `T` — возврат; режим `--shot` без окна (по образцу `foxrun`) для проверки кадрами. Абзацы: `PLAN_PHYSICS_NAV.md`, `PLAN_EXECUTION.md` (B4 «ragdoll»), `CHANGELOG.md`, `docs/CRATES.md` (регенерация `gen_crates_doc.py`).

## Границы
- **Читать:** `redox_physics/src/{joints,joint_component,context,components,layer_config,contacts}.rs`, `tests/joints*.rs`, `redox_anim/src/{skeleton,animator,graph}.rs`, `redox_render/src/skinning.rs` (приём поз), `foxrun/fox.rs`.
- **Создать:** `redox_physics/src/ragdoll.rs`, `tests/{joint_limits,ragdoll}.rs`, `redox_core/tests/ragdoll_pose.rs`, `redox_core/src/ragdoll_system.rs`, `examples/ragdoll_demo.rs`.
- **Менять:** `redox_physics/src/{joints,lib}.rs` (BallLimited — аддитивно), `redox_core/src/plugin.rs`, `redox_render/src/skinning.rs` (вход внешних поз).
- **Не трогать:** контроллер персонажа (PHY-01…03), существующие варианты `JointKind`.

## Проверка
`cargo fmt`, `cargo clippy -p redox_physics -p redox_core -p redox_anim --all-targets -- -D warnings`, `cargo test -p redox_physics --test joint_limits --test ragdoll`, `cargo test -p redox_core --test ragdoll_pose`, `cargo check --workspace --all-targets`, `python scripts/check_file_sizes.py`, `python scripts/gen_crates_doc.py --check`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. В `redox_physics` одновременно не больше двух агентов. Физические числа (углы, массы) в тестах — из спеки; если решатель не держит лимит с допуском — **остановись и сообщи измеренное значение** (так было с моторами).

## Закрытие (финальная волна)

Исход обязателен: ✅ / ⛔ / 📌 (`../FINAL_WAVE.md` §1). Агенту разрешён `cargo check -p <крейт> --all-targets` в своём worktree; тесты, clippy и замеры — у координатора. Шаг, у которого приёмочный тест после доработки красный, не остаётся «частично»: агент сообщает измеренное число, исход назначает координатор.

- **Осталось (шаг 5):** пример `ragdoll_demo`; проверка единичных кадров суставов в `joint_desc` (вторая гипотеза подпрыгивания); если нужен кадр — добавить в `JointDesc`. Тест «ragdoll не подпрыгивает» обязателен.
- **Приёмка:** пример запускается, тест зелёный.

## Исход шага 5 (2026-10-30)

- ✅ `crates/redox_core/examples/ragdoll_window.rs`: окно, `R` — ragdoll, `T` — возврат в анимацию; `--shot <png>` (без окна), `--selftest` (без окна, код 0/1). Переходы вынесены в `RagdollToggle` (юнит-тесты в примере).
- 📌 Отклонение от спеки: фигура не лиса, а «коробочный человек» (один куб на тело шаблона). Скелет Fox не гуманоидный (24 кости, не Mixamo), `RagdollTemplate::humanoid` на нём не строится.
- Проверено: `cargo check -p redox_core --examples`; `--selftest` возвращает 0; подъём центра масс после активации из покоя ≤ 0.1 м (тот же критерий, что в `ragdoll_activation`); гипсы тела ниже 0.4 м через 3 с.
- Не проверено глазами в живом окне (в агенте нет дисплея); `--shot` написан, его результат не просмотрен координатором.
- Тест «ragdoll не подпрыгивает» — существующий `tests/ragdoll_activation.rs`.
