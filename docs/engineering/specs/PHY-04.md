# PHY-04 — Компонент `Joint` в ECS

- **Размер:** S. **Крейт:** `redox_physics`. **Зависит от:** ничего. **Блокирует:** ничего.
- **Параллельно с:** PHY-01…03 (другие файлы), но не с другим заданием, трогающим `sync.rs` или `lib.rs`.
- **Модель:** haiku.

## Суть
Шарниры создаются только вручную: `PhysicsContext::add_joint(a, b, desc)` (`joints.rs:303`) возвращает `JointId`, который надо
хранить самому. В сцене, редакторе и скриптах нужен компонент, как у `RigidBody`/`Collider`.

## Границы
- **Читать:** `joints.rs` (`JointDesc`, `JointId`, `JointBroken`, `add_joint`, `remove_joint`, `joint_exists`, `drain_joint_broken`),
  `sync.rs::tick_physics` (строка 168), `lib.rs` (реэкспорты), `tests/joints.rs` (образец сборки мира).
- **Создать:** `src/joint_component.rs`, `tests/joint_component.rs`.
- **Менять:** `src/sync.rs` (одна строка вызова), `src/lib.rs` (`mod` и реэкспорт). Больше ничего.

## Решение (`joint_component.rs`)
```rust
use redox_ecs::{Entity, World};
use crate::joints::{JointDesc, JointId};

/// Joins this entity's body with the body of `other`. Created in the physics world by [`sync_joints`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Joint { pub other: Entity, pub desc: JointDesc }

/// Added by [`sync_joints`] to an entity whose `Joint` exists in the physics world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JointLink(pub JointId);

/// Reconciles `Joint` components with the physics world; call once per physics tick before the step.
pub fn sync_joints(world: &mut World);
```
`sync_joints` (без `unwrap`; ошибки `add_joint` логируются `log::warn!`, компонент `Joint` при этом **остаётся**, повтор на следующем тике — не более
одной попытки на тик):
1. Достать `PhysicsContext` через `remove_resource`, вернуть в конце (как `character.rs`). Нет ресурса → `return`.
2. Собрать `Vec<(Entity, Joint)>` из `world.query::<(Entity, &Joint)>()` для сущностей **без** `JointLink`
   (`get_component::<JointLink>(e).is_none()`); для каждой `add_joint(e, joint.other, joint.desc)` → при `Ok(id)` `add_component(e, JointLink(id))`.
3. Собрать `Vec<(Entity, JointId)>` из `JointLink`; если у сущности нет `Joint` **или** `!joint_exists(id)`: `remove_joint(id)` (если ещё есть), `remove_component::<JointLink>(e)`;
   если не было `Joint` — ничего больше; если сустав не существует (разорван) — также `remove_component::<Joint>(e)`.
4. Разрывы: `drain_joint_broken()` читать **в `tick_physics`** (там уже дренируются остальные очереди) и слать `Events<JointBroken>` если ресурс есть
   (`JointBroken` уже `Copy`). `sync_joints` после этого видит `!joint_exists` и чистит компоненты. Поэтому порядок в `tick_physics`:
   `sync_joints` → `move_character_controllers` → `step_physics` → дренаж событий (+ новый дренаж разрывов).
Значит `sync_joints` вызывается первой строкой `tick_physics`, а дренаж разрывов добавляется рядом с остальными.

## Пошагово
1. Тесты → красные (не компилируются: допустимо, это спецификация-«каркас»; сделать `todo!()` в `sync_joints`, чтобы компилировались).
2. Реализовать, подключить в `sync.rs`/`lib.rs` (`pub use joint_component::{Joint, JointLink, sync_joints};`).

## Тесты (`tests/joint_component.rs`; сборка мира — по `tests/joints.rs`)
```rust
#[test]
fn a_joint_component_creates_a_joint_once() {
    // two dynamic balls a, b (gravity 0); world.add_component(a, Joint { other: b, desc: JointDesc::new(JointKind::Fixed) });
    // tick_physics twice
    // assert: a has JointLink; ctx.joint_exists(link.0); only one joint (the id is the same after the second tick)
}
#[test]
fn removing_the_component_removes_the_joint() {
    // after creation: world.remove_component::<Joint>(a); tick; assert !ctx.joint_exists(id) and a has no JointLink
}
#[test]
fn a_broken_joint_clears_both_components_and_sends_the_event() {
    // JointDesc { break_force: Some(1.0), ..JointDesc::new(JointKind::Fixed) }; pull b away with a velocity of 50;
    // Events::<JointBroken> in the world; tick until an event arrives (at most 60 ticks);
    // assert: no Joint, no JointLink on a; one event with a, b.
}
#[test]
fn a_joint_to_an_entity_without_a_body_is_kept_and_retried() {
    // other = world.spawn() with no RigidBody: add_joint fails, Joint stays, no JointLink; tick does not panic.
    // Then give `other` a body (as spawn_body does) and tick: JointLink appears.
}
```

## Проверка
`cargo test -p redox_physics`, `cargo clippy -p redox_physics --all-targets -- -D warnings`, `cargo fmt --check`.

## Для исполнителя
`git merge --ff-only main`. Не используй `unwrap` вне тестов. Готово — когда четыре теста зелёные и ничего из `joint*.rs` тестов не сломалось.
