# PHY-02 — Контроллер персонажа: ступени, склоны и прилипание к земле

- **Статус:** выполнено 2026-10-29, принято в `main`. Тест прогулки по ступени сокращён до 70 кадров (при 120 персонаж проходил весь край ступени и сходил вниз: ошибка спецификации).

- **Размер:** S. **Крейт:** `redox_physics`. **Зависит от:** PHY-01 (оба правят `character.rs` и `components.rs`: выполнять строго после). **Блокирует:** PHY-03.
- **Модель:** haiku.

## Суть
`move_character_controllers` создаёт `RapierKcc::default()` на каждого персонажа: автоступени выключены, угол склона и
прилипание не настраиваются. Тест `walking_into_a_low_step_climbs_it` (`tests/character_v2.rs`) показывает: персонаж
останавливается перед ступенью высотой 0,2 (x = 0,69).

## Границы
- **Читать:** `character.rs`, `components.rs`, `tests/character_v2.rs`; документацию rapier `KinematicCharacterController`
  (`rapier3d::control`; поля `max_slope_climb_angle`, `min_slope_slide_angle`, `autostep: Option<CharacterAutostep>`,
  `snap_to_ground: Option<CharacterLength>`, `offset`). Точные имена и типы сверить с `cargo doc -p rapier3d --open`
  или исходниками в `~/.cargo/registry/src/*/rapier3d-0.36*/src/control/character_controller.rs`.
- **Менять:** `components.rs`, `character.rs`, `tests/character_v2.rs`.
- **Не трогать:** остальные файлы и крейты.

## Решение
Новая структура настроек и поле в компоненте:

```rust
/// Movement tuning of a [`CharacterController`]; lengths in world units, angles in radians.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CharacterSettings {
    /// Highest obstacle climbed without jumping; 0.0 disables autostep.
    pub step_height: f32,
    /// Free room in front of the character needed to start a step (rapier `min_width`).
    pub step_min_width: f32,
    /// Steepest slope that can be walked up.
    pub max_slope_angle: f32,
    /// Distance below the feet at which the character is pulled back to the ground; 0.0 disables.
    pub snap_to_ground: f32,
}
impl Default for CharacterSettings { /* step_height 0.3, step_min_width 0.1, max_slope_angle 0.785398 (45°), snap_to_ground 0.2 */ }
```
В `CharacterController` добавить `pub settings: CharacterSettings` (Default — как выше). В `character.rs` заменить
`RapierKcc::default()` на функцию `fn build_kcc(s: &CharacterSettings) -> RapierKcc`, которая выставляет:
`max_slope_climb_angle = s.max_slope_angle`, `autostep = (s.step_height > 0.0).then(|| CharacterAutostep { max_height: CharacterLength::Absolute(s.step_height), min_width: CharacterLength::Absolute(s.step_min_width), include_dynamic_bodies: false })`,
`snap_to_ground = (s.snap_to_ground > 0.0).then(|| CharacterLength::Absolute(s.snap_to_ground))`. В кортеж `entities` добавить `settings`.
Прыжок (PHY-01): пока `vertical_velocity > 0.0`, `snap_to_ground` отключать (иначе притянет вниз сразу после прыжка) —
создавать `kcc` с `snap_to_ground = None` в этом случае.

## Пошагово
1. Снять `#[ignore]` с `walking_into_a_low_step_climbs_it`, убедиться, что красный.
2. Добавить `CharacterSettings`, поле, `build_kcc`; подключить; тест становится зелёным.
3. Добавить тесты (ниже). Экспортировать `CharacterSettings` из `components.rs` в `lib.rs` **нельзя** (запрещено трогать `lib.rs`):
   тесты берут тип как `redox_physics::components::CharacterSettings`; в отчёте написать, что реэкспорт в корень остаётся владельцу (я добавлю).

## Тесты
```rust
use redox_physics::components::CharacterSettings;

#[test]
fn a_step_higher_than_step_height_blocks_the_character() {
    let mut world = World::new();
    world.insert_resource(PhysicsContext::new(Vec3::ZERO));
    spawn_body(&mut world, Vec3::new(0.0, -0.5, 0.0), RigidBodyBuilder::fixed(),
        ColliderBuilder::cuboid(20.0, 0.5, 5.0));
    // step top at 0.6, higher than the default 0.3
    spawn_body(&mut world, Vec3::new(3.0, 0.3, 0.0), RigidBodyBuilder::fixed(),
        ColliderBuilder::cuboid(2.0, 0.3, 2.0));
    let player = spawn_player(&mut world, Vec3::new(0.0, 0.75, 0.0));
    for _ in 0..70 {
        world.get_component_mut::<CharacterController>(player).unwrap()
            .desired_translation = Vec3::new(3.0 * DT, 0.0, 0.0);
        tick_physics(&mut world, DT);
    }
    assert!(position(&world, player).x < 1.0, "x = {}", position(&world, player).x);
}

#[test]
fn walking_down_a_ramp_keeps_the_feet_on_the_ground() {
    // Ramp falling 0.3 over 2 units of x (about 8.5 degrees). Without snapping the character
    // would hover and be reported as not grounded in some frames.
    let mut world = World::new();
    world.insert_resource(PhysicsContext::new(Vec3::ZERO));
    let ramp = ColliderBuilder::convex_hull(&[
        Vec3::new(0.0, 0.0, -2.0), Vec3::new(0.0, 0.0, 2.0),
        Vec3::new(4.0, -0.6, -2.0), Vec3::new(4.0, -0.6, 2.0),
        Vec3::new(0.0, -1.0, -2.0), Vec3::new(0.0, -1.0, 2.0),
        Vec3::new(4.0, -1.0, -2.0), Vec3::new(4.0, -1.0, 2.0),
    ]).unwrap();
    spawn_body(&mut world, Vec3::ZERO, RigidBodyBuilder::fixed(), ramp);
    let player = spawn_player(&mut world, Vec3::new(0.2, 0.75, 0.0));
    for _ in 0..20 { tick_physics(&mut world, DT); }
    let mut grounded_frames = 0;
    for _ in 0..60 {
        world.get_component_mut::<CharacterController>(player).unwrap()
            .desired_translation = Vec3::new(2.0 * DT, 0.0, 0.0);
        tick_physics(&mut world, DT);
        if world.get_component::<CharacterController>(player).unwrap().grounded { grounded_frames += 1; }
    }
    assert!(grounded_frames >= 58, "grounded in {grounded_frames} of 60 frames");
}

#[test]
fn zero_step_height_disables_autostep() {
    // same setup as walking_into_a_low_step_climbs_it, but settings.step_height = 0.0 -> x stays below 1.0
}
```
(Третий тест агент дописывает по образцу первого, `step_height = 0.0`.) Если у `ColliderBuilder::convex_hull`
иная сигнатура в rapier 0.36, использовать `crate::convex::ConvexHull::from_points` (см. `convex.rs`) или `ColliderBuilder::trimesh`.

## Проверка
`cargo test -p redox_physics --test character_v2`, `cargo test -p redox_physics --lib`, `cargo clippy -p redox_physics --all-targets -- -D warnings`.

## Для исполнителя
`git merge --ff-only main` первым шагом. Если после PHY-01 полей в `CharacterController` больше — не переписывай литералы заново,
используй `..Default::default()`. Готово, когда оба теста ступеней и тест рампы зелёные. Если тест рампы упирается в решатель
(как было с моторами) — не подгоняй число, а сообщи, что именно измерено.
