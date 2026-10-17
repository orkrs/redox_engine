//! `World::on_add` / `World::on_remove` component hooks.

#![allow(clippy::unwrap_used)]

use std::sync::{Arc, Mutex};

use redox_ecs::World;

#[derive(Debug, PartialEq)]
struct Health(i32);
struct Tag;

type Log = Arc<Mutex<Vec<String>>>;

fn hooked_world() -> (World, Log) {
    let log: Log = Arc::default();
    let mut world = World::new();
    let l = log.clone();
    world.on_add::<Health>(move |_, _| l.lock().unwrap().push("add".into()));
    let l = log.clone();
    world.on_remove::<Health>(move |w, e| {
        let hp = w.get_component::<Health>(e).map(|h| h.0);
        l.lock().unwrap().push(format!("remove {hp:?}"));
    });
    (world, log)
}

#[test]
fn add_hook_runs_for_add_component_and_spawn_but_not_overwrite() {
    let (mut world, log) = hooked_world();
    let a = world.spawn_bundle((Health(1), Tag));
    let b = world.spawn();
    world.add_component(b, Health(2));
    world.add_component(b, Health(3)); // overwrite
    world.add_component(a, Tag); // other type
    assert_eq!(log.lock().unwrap().len(), 2);
}

#[test]
fn remove_hook_sees_the_value_on_remove_and_despawn() {
    let (mut world, log) = hooked_world();
    let a = world.spawn_bundle((Health(7), Tag));
    assert!(world.remove_component::<Health>(a));
    let b = world.spawn_bundle((Health(9), Tag));
    world.despawn(b);
    let entries = log.lock().unwrap().clone();
    assert_eq!(entries, ["add", "remove Some(7)", "add", "remove Some(9)"]);
}

#[test]
fn removing_the_last_component_fires_the_hook_once() {
    let (mut world, log) = hooked_world();
    let a = world.spawn_bundle((Health(5),));
    world.remove_component::<Health>(a);
    let removes = log
        .lock()
        .unwrap()
        .iter()
        .filter(|s| s.starts_with("remove"))
        .count();
    assert_eq!(removes, 1);
}

#[test]
fn hooks_may_mutate_the_world() {
    let mut world = World::new();
    world.on_add::<Health>(|w, e| w.add_component(e, Tag));
    let e = world.spawn_bundle((Health(1),));
    assert!(world.get_component::<Tag>(e).is_some());
}

#[test]
fn worlds_without_hooks_are_unaffected() {
    let mut world = World::new();
    let e = world.spawn_bundle((Health(1),));
    world.despawn(e);
    assert!(world.get_component::<Health>(e).is_none());
}
