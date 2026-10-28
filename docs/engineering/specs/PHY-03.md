# PHY-03 — Контроллер персонажа: подвижные платформы

- **Размер:** M. **Крейт:** `redox_physics`. **Зависит от:** PHY-01, PHY-02. **Блокирует:** ничего.
- **Модель:** sonnet (нужно разобраться в порядке систем и кинематике rapier).

## Суть
Персонаж, стоящий на движущейся кинематической платформе, не двигается вместе с ней: контроллер знает только о
собственном сдвиге. Нужно переносить персонажа на сдвиг земли под ним за шаг.

## Границы
- **Читать:** `character.rs`, `sync.rs` (как обновляется трансформ кинематических тел: `Kinematic`, строки 30 и 96),
  `context.rs` (`set_rigid_body_transform`, доступ к `bodies`/`colliders`), `components.rs`.
- **Менять:** `character.rs`, `components.rs` (одно поле), новый тест `tests/character_platform.rs`.
- **Не трогать:** `sync.rs`, `context.rs`, `lib.rs`.

## Решение
Платформа — сущность с `RigidBody` (кинематика, позиция задаётся через `Transform` + `Kinematic`) и `Collider`.
Идея: запомнить, на какой коллайдер опирался персонаж, и сдвиг этого коллайдера за шаг.

В `CharacterController` добавить:
```rust
/// Collider the character stood on after the last move (the one `grounded` refers to). Controller state.
pub ground_collider: Option<rapier3d::geometry::ColliderHandle>,
```
(Тип `ColliderHandle` из `rapier3d::prelude`; `Default` — `None`.)

В `move_character_controllers`:
1. В коллбэке `move_shape(.., |collision| ..)` (сейчас `|_| {}`) запомнить `collision.handle` того столкновения, нормаль которого
   смотрит вверх (`collision.hit.normal2.y` или `normal1` — проверить по документации rapier: нужен вектор, направленный от поверхности к
   персонажу; условие «угол с вертикалью < `max_slope_angle`»). Если `movement.grounded` и такого столкновения нет — `None`.
2. Для платформы хранить позицию коллайдера прошлого шага: в `PhysicsContext` ничего не добавлять; вместо этого в
   `CharacterController` держать `ground_pose: Option<(ColliderHandle, Vec3)>` — позиция коллайдера на момент прошлого шага.
   Перед `move_shape` текущего шага: если `ground_pose` есть и handle совпадает с текущей позицией коллайдера `physics.colliders.get(h)`,
   то `carry = current_translation - stored_translation` и `wish += carry` (только горизонталь и вертикаль вместе, платформа может ехать вверх/вниз).
3. После шага сохранить `ground_pose = Some((handle, current_translation_of_that_collider))`.
Порядок в `tick_physics` (проверено, `sync.rs:168`): `move_character_controllers` → `step_physics` → события. Контроллер работает **до** шага Rapier, поэтому
позиция коллайдера платформы в этот момент — позиция после прошлого шага; перенос получается с запаздыванием на один кадр, но без накопления ошибки
(сдвиг считается как разность двух последовательных позиций). Не пытаться «догонять» шаг.

## Пошагово
1. Записать в комментарии кода порядок вызовов из `tick_physics` (см. выше).
2. Реализовать запоминание земли и перенос.
3. Тест (ниже) сначала красный, потом зелёный.

## Тест (`tests/character_platform.rs`, хелперы скопировать из `character_v2.rs`)
```rust
#[test]
fn standing_on_a_moving_platform_carries_the_character() {
    let mut world = World::new();
    world.insert_resource(PhysicsContext::new(Vec3::ZERO));
    let platform = spawn_body(&mut world, Vec3::new(0.0, -0.25, 0.0),
        RigidBodyBuilder::kinematic_position_based(), ColliderBuilder::cuboid(2.0, 0.25, 2.0));
    world.add_component(platform, Kinematic);
    let player = spawn_player(&mut world, Vec3::new(0.0, 0.75, 0.0)); // as in character_v2
    for _ in 0..30 { tick_physics(&mut world, DT); }
    let start = position(&world, player);
    for _ in 0..60 {
        world.get_component_mut::<Transform>(platform).unwrap().translation.x += 1.0 * DT; // 1 unit/s
        tick_physics(&mut world, DT);
    }
    let end = position(&world, player);
    assert!((end.x - start.x - 1.0).abs() < 0.1, "moved {} of 1.0", end.x - start.x);
    assert!((end.y - start.y).abs() < 0.05);
}

#[test]
fn leaving_the_platform_stops_the_carry() {
    // platform moves 1 u/s for 1 s, the player walks off sideways (z) at 2 u/s during the second half;
    // after leaving, x must stop changing (tolerance 0.05 over the last 20 frames).
}
```
Второй тест агент дописывает по образцу.

## Проверка
`cargo test -p redox_physics` целиком (тесты `kcc_tests` и `character_v2` не должны сломаться), clippy, fmt.

## Для исполнителя
`git merge --ff-only main`. Если порядок систем делает перенос невозможным без правки `sync.rs` — не правь, остановись и
опиши в отчёте, что именно нужно поменять (я решу).
