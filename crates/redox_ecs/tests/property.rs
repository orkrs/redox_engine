//! Property tests: random spawn / despawn / add / remove sequences must agree with a
//! trivial reference model and keep the archetype storage consistent.

use proptest::prelude::*;
use redox_ecs::{Entity, Query, World};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
struct A(u32);
#[derive(Clone, Debug, PartialEq)]
struct B(u32);
#[derive(Clone, Debug, PartialEq)]
struct C(u32);

#[derive(Clone, Debug)]
enum Op {
    Spawn,
    Despawn(usize),
    AddA(usize, u32),
    AddB(usize, u32),
    AddC(usize, u32),
    RemoveA(usize),
    RemoveB(usize),
    RemoveC(usize),
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        Just(Op::Spawn),
        any::<usize>().prop_map(Op::Despawn),
        (any::<usize>(), any::<u32>()).prop_map(|(i, v)| Op::AddA(i, v)),
        (any::<usize>(), any::<u32>()).prop_map(|(i, v)| Op::AddB(i, v)),
        (any::<usize>(), any::<u32>()).prop_map(|(i, v)| Op::AddC(i, v)),
        any::<usize>().prop_map(Op::RemoveA),
        any::<usize>().prop_map(Op::RemoveB),
        any::<usize>().prop_map(Op::RemoveC),
    ]
}

#[derive(Default, Clone)]
struct Model {
    a: Option<u32>,
    b: Option<u32>,
    c: Option<u32>,
}

impl Model {
    fn is_empty(&self) -> bool {
        self.a.is_none() && self.b.is_none() && self.c.is_none()
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn world_matches_reference_model(ops in prop::collection::vec(op_strategy(), 1..200)) {
        let mut world = World::new();
        let mut live: Vec<Entity> = Vec::new();
        let mut model: HashMap<Entity, Model> = HashMap::new();

        for op in ops {
            let pick = |i: usize, live: &[Entity]| {
                if live.is_empty() { None } else { Some(live[i % live.len()]) }
            };
            match op {
                Op::Spawn => {
                    let e = world.spawn();
                    live.push(e);
                    model.insert(e, Model::default());
                }
                Op::Despawn(i) => {
                    if let Some(e) = pick(i, &live) {
                        world.despawn(e);
                        live.retain(|x| *x != e);
                        model.remove(&e);
                    }
                }
                Op::AddA(i, v) => {
                    if let Some(e) = pick(i, &live) {
                        world.add_component(e, A(v));
                        model.get_mut(&e).unwrap().a = Some(v);
                    }
                }
                Op::AddB(i, v) => {
                    if let Some(e) = pick(i, &live) {
                        world.add_component(e, B(v));
                        model.get_mut(&e).unwrap().b = Some(v);
                    }
                }
                Op::AddC(i, v) => {
                    if let Some(e) = pick(i, &live) {
                        world.add_component(e, C(v));
                        model.get_mut(&e).unwrap().c = Some(v);
                    }
                }
                Op::RemoveA(i) => {
                    if let Some(e) = pick(i, &live) {
                        let had = model[&e].a.is_some();
                        prop_assert_eq!(world.remove_component::<A>(e), had);
                        model.get_mut(&e).unwrap().a = None;
                    }
                }
                Op::RemoveB(i) => {
                    if let Some(e) = pick(i, &live) {
                        let had = model[&e].b.is_some();
                        prop_assert_eq!(world.remove_component::<B>(e), had);
                        model.get_mut(&e).unwrap().b = None;
                    }
                }
                Op::RemoveC(i) => {
                    if let Some(e) = pick(i, &live) {
                        let had = model[&e].c.is_some();
                        prop_assert_eq!(world.remove_component::<C>(e), had);
                        model.get_mut(&e).unwrap().c = None;
                    }
                }
            }
        }

        // Every entity that still holds a component reads back exactly the model's
        // values (an entity whose last component was removed may or may not linger).
        for e in &live {
            let m = &model[e];
            if m.is_empty() {
                continue;
            }
            prop_assert_eq!(world.get_component::<A>(*e).map(|c| c.0), m.a);
            prop_assert_eq!(world.get_component::<B>(*e).map(|c| c.0), m.b);
            prop_assert_eq!(world.get_component::<C>(*e).map(|c| c.0), m.c);
        }

        // No entity appears twice in the archetype tables.
        let mut all: Vec<(u32, u32)> = Vec::new();
        for (_, _, entities) in world.archetype_iter() {
            all.extend(entities.iter().map(|e| (e.id(), e.generation())));
        }
        let total = all.len();
        all.sort_unstable();
        all.dedup();
        prop_assert_eq!(all.len(), total, "entity stored in more than one row");

        // Queries see every holder exactly once.
        let with_a = model.values().filter(|m| m.a.is_some()).count();
        prop_assert_eq!(Query::<&A>::new().iter(&world).count(), with_a);
    }
}
