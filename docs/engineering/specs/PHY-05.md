# PHY-05 — Правила комбинирования трения и упругости (`CombineRule`)

- **Размер:** S. **Крейт:** `redox_physics`. **Зависит от:** ничего. **Блокирует:** ничего.
- **Модель:** haiku.

## Суть
`PhysicsMaterial` (`contacts.rs:60`) задаёт трение и упругость, а при контакте двух поверхностей Rapier усредняет значения (`Average`).
Лёд с резиной, мячик на мягком полу: «побеждает меньшее/большее» нужно игре. Rapier это умеет (`Collider::set_friction_combine_rule`,
`set_restitution_combine_rule`, `CoefficientCombineRule`), движок не выставляет.

## Границы
- **Читать:** `contacts.rs` (строки 55–75 и 160–190: `PhysicsMaterial`, `set_physics_material`, `for_each_collider_of`), `tests/contact_events.rs` (образец сборки мира),
  в rapier: `rapier3d-0.36*/src/dynamics/coefficient_combine_rule.rs` (имена вариантов: Average, Min, Multiply, Max, ClampedSum, GeometricMean).
- **Менять:** `contacts.rs`, `lib.rs` (реэкспорт `CombineRule`), новый `tests/combine_rules.rs`.
- **Не трогать:** `PhysicsMaterial` как структуру (литералы `PhysicsMaterial { friction, restitution }` есть в тестах и играх — **поля не добавлять**).

## Решение
```rust
/// How the coefficients of two touching surfaces become one. When the two colliders ask for different rules, the one
/// listed later wins (Average < Min < Multiply < Max), as in Rapier.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CombineRule { #[default] Average, Min, Multiply, Max }

impl PhysicsContext {
    /// Sets how the friction and restitution of `entity`'s colliders are combined with the other collider of a contact.
    /// Default for every collider: `Average`.
    pub fn set_combine_rules(&mut self, entity: Entity, friction: CombineRule, restitution: CombineRule)
        -> Result<(), PhysicsError>;
}
```
Реализация: `for_each_collider_of(entity, |c| { c.set_friction_combine_rule(map(friction)); c.set_restitution_combine_rule(map(restitution)); })`, где `map` — `match` в
`CoefficientCombineRule`. Поправить комментарий поля `friction` («combined … by averaging» → «по умолчанию усредняется, см. `CombineRule`»).

## Тесты (`tests/combine_rules.rs`)
```rust
fn drop_height_ratio(ball_rule: CombineRule) -> f32 {
    // floor: fixed cuboid top at y = 0, restitution 0.0; ball: dynamic ball r = 0.25 at y = 2.25 (bottom at 2.0), restitution 1.0,
    // ctx gravity (0,-9.81,0). set_physics_material for both; set_combine_rules(ball, Average, ball_rule).
    // step 1/120 s up to 3 s; record the highest y of the ball centre after the first contact (velocity turned from negative to positive);
    // return (peak - 0.25) / 2.0   // rebound height / drop height
}
#[test] fn the_default_rule_averages_the_restitution() {
    let r = drop_height_ratio(CombineRule::Average);
    assert!(r > 0.1 && r < 0.45, "expected about 0.25, got {r}");   // e = 0.5 -> e^2 = 0.25
}
#[test] fn max_rule_makes_the_ball_bounce_back_nearly_fully() {
    let r = drop_height_ratio(CombineRule::Max);
    assert!(r > 0.6, "expected close to 1.0, got {r}");              // e = 1.0
}
#[test] fn min_rule_with_a_dead_floor_removes_the_bounce() {
    let r = drop_height_ratio(CombineRule::Min);
    assert!(r < 0.05, "expected no bounce, got {r}");                // e = 0.0
}
#[test] fn unknown_entities_are_an_error() {
    // ctx.set_combine_rules(world.spawn(), Max, Max) == Err(PhysicsError::EntityNotFound)
}
#[test] fn a_stronger_rule_on_either_collider_wins() {
    // rule Max on the FLOOR and Average on the ball gives the same big bounce as Max on the ball.
}
```
Допуски широкие, потому что решатель Rapier теряет часть энергии; если реальное число выходит за допуск, агент **не подгоняет** его, а сообщает измеренные значения.

## Проверка
`cargo test -p redox_physics`, clippy, fmt.

## Для исполнителя
`git merge --ff-only main`. Файл `lib.rs` — только строка реэкспорта (не пересекаться с PHY-04, который правит тот же файл: запускать после PHY-04 или согласовать порядок слияния).
