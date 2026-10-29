# PHY-01 — Контроллер персонажа: вертикальная скорость, гравитация и прыжок

- **Статус:** выполнено 2026-10-29, принято в `main`.

- **Размер:** S. **Крейт:** `redox_physics`. **Зависит от:** ничего. **Блокирует:** PHY-02, PHY-03.
- **Модель:** haiku (тест уже готов, файлов два).

## Суть
`move_character_controllers` (`crates/redox_physics/src/character.rs:11`) каждый шаг вычитает из желаемого сдвига
`gravity * dt` (строка `wish.y -= gravity * dt`). Скорость падения не накапливается: за секунду персонаж падает
на 20 единиц вместо 10 (измерено: 19,99994), и прыжка нет. Нужна вертикальная скорость, которая хранится в компоненте.

## Границы
- **Читать:** `crates/redox_physics/src/character.rs`, `components.rs` (структура `CharacterController`, строки 36–55),
  `tests/character_v2.rs`.
- **Менять:** `components.rs`, `character.rs`, `tests/character_v2.rs` (снять `#[ignore]` с `free_fall_accelerates_like_real_gravity`,
  добавить тесты ниже).
- **Не трогать:** `sync.rs`, `context.rs`, `lib.rs`, любые другие крейты, поведение `desired_translation`.

## Решение
В `CharacterController` добавить два публичных поля и метод (поле `Default` — нули):

```rust
pub struct CharacterController {
    pub desired_translation: Vec3,
    pub gravity: f32,
    pub grounded: bool,
    /// Vertical speed in units per second, positive is up. Owned by the controller.
    pub vertical_velocity: f32,
    /// Speed of a jump set by `jump`, units per second.
    pub jump_speed: f32,
}
impl CharacterController {
    /// Starts a jump if the character stood on the ground after the last move. Returns whether it started.
    pub fn jump(&mut self) -> bool;
}
```

`Default`: `vertical_velocity: 0.0`, `jump_speed: 7.0`. Существующий литерал в `character.rs` (тест
`character_lands_on_floor`) собирает структуру по полям — добавить `..Default::default()`.

Алгоритм шага в `move_character_controllers` (вместо `wish.y -= gravity * dt`):
1. До движения: `vertical_velocity -= gravity * dt` (полу-неявный Эйлер).
2. `wish.y += vertical_velocity * dt` (прибавляется к желаемому сдвигу, `desired_translation.y` сохраняется).
3. После `move_shape`: если `movement.grounded` и `vertical_velocity < 0.0` → `vertical_velocity = 0.0`.
   Если по вертикали сдвиг оказался заметно меньше желаемого при движении вверх (удар головой: `vertical_velocity > 0.0`
   и `movement.translation.y < wish.y - 1e-4`) → `vertical_velocity = 0.0`.
4. `jump()`: если `grounded` → `vertical_velocity = jump_speed; grounded = false; true`, иначе `false`.
5. Результат (`vertical_velocity`) записывается обратно в компонент в цикле применения результатов (рядом с `cc.grounded = grounded`);
   для этого в кортеж `results` добавить новое значение скорости.

Ошибки: никаких `unwrap`; все `Option` обрабатываются `let Some(..) else { continue }` как сейчас.

## Пошагово
1. Снять `#[ignore]` с `free_fall_accelerates_like_real_gravity`; запустить — красный (20 единиц).
2. Добавить поля и `jump`, поправить литерал в `character.rs`.
3. Переписать вертикаль в `move_character_controllers`.
4. Добавить тесты (ниже), довести до зелёного.

## Тесты (добавить в `tests/character_v2.rs`; хелперы `spawn_body`, `spawn_player`, `position` там уже есть)
```rust
#[test]
fn jump_rises_then_lands_back_on_the_floor() {
    let mut world = World::new();
    world.insert_resource(PhysicsContext::new(Vec3::ZERO));
    spawn_body(&mut world, Vec3::new(0.0, -0.5, 0.0), RigidBodyBuilder::fixed(),
        ColliderBuilder::cuboid(10.0, 0.5, 10.0));
    let player = spawn_player(&mut world, Vec3::new(0.0, 0.75, 0.0));
    for _ in 0..30 { tick_physics(&mut world, DT); } // settle
    assert!(world.get_component::<CharacterController>(player).unwrap().grounded);
    let rest = position(&world, player).y;
    assert!(world.get_component_mut::<CharacterController>(player).unwrap().jump());
    let mut peak = rest;
    for _ in 0..30 { tick_physics(&mut world, DT); peak = peak.max(position(&world, player).y); }
    // v^2 / 2g = 49 / 40 = 1.225
    assert!((peak - rest - 1.2).abs() < 0.2, "peak rise {}", peak - rest);
    for _ in 0..120 { tick_physics(&mut world, DT); }
    assert!((position(&world, player).y - rest).abs() < 0.05);
    assert!(world.get_component::<CharacterController>(player).unwrap().grounded);
}

#[test]
fn jump_in_the_air_is_refused() {
    let mut cc = CharacterController::default();
    assert!(!cc.jump(), "not grounded yet");
    cc.grounded = true;
    assert!(cc.jump());
    assert!(!cc.jump(), "already left the ground");
}

#[test]
fn a_low_ceiling_stops_the_rise() {
    let mut world = World::new();
    world.insert_resource(PhysicsContext::new(Vec3::ZERO));
    spawn_body(&mut world, Vec3::new(0.0, -0.5, 0.0), RigidBodyBuilder::fixed(),
        ColliderBuilder::cuboid(10.0, 0.5, 10.0));
    // underside of the ceiling at y = 1.6
    spawn_body(&mut world, Vec3::new(0.0, 2.1, 0.0), RigidBodyBuilder::fixed(),
        ColliderBuilder::cuboid(10.0, 0.5, 10.0));
    let player = spawn_player(&mut world, Vec3::new(0.0, 0.75, 0.0));
    for _ in 0..30 { tick_physics(&mut world, DT); }
    world.get_component_mut::<CharacterController>(player).unwrap().jump();
    for _ in 0..20 { tick_physics(&mut world, DT); }
    let top = position(&world, player).y + 0.7; // capsule half height
    assert!(top <= 1.65, "head went through the ceiling: {top}");
}
```

## Проверка
`cargo test -p redox_physics --test character_v2` (+ `cargo test -p redox_physics --lib` — старые тесты `kcc_tests` остаются зелёными).
Граничные случаи: `dt = 0` не меняет скорость; очень большой `dt` (0,25) не пробивает пол (в тесте `character_lands_on_floor` не требуется, достаточно не паниковать).

## Для исполнителя
Начни с `git merge --ff-only main`. Затем шаги выше. Готово, когда: тесты `character_v2` (кроме PHY-02) зелёные,
`cargo clippy -p redox_physics --all-targets -- -D warnings` и `cargo fmt --check` чисты. Закоммить локально; не пушить.
