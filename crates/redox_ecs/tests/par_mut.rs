//! Parallel mutable iteration.

use redox_ecs::{Changed, Entity, Query, With, Without, World};

#[derive(Clone, Copy, Debug, PartialEq)]
struct Pos(f32);
#[derive(Clone, Copy, Debug, PartialEq)]
struct Vel(f32);
struct Frozen;

fn world(n: usize) -> World {
    let mut w = World::new();
    for i in 0..n {
        w.spawn_bundle((Pos(i as f32), Vel(1.0)));
    }
    w
}

#[test]
fn every_row_is_visited_exactly_once() {
    let mut w = world(10_000);
    Query::<(&mut Pos, &Vel)>::new().par_for_each_mut(&mut w, |(p, v)| p.0 += v.0);
    let mut seen: Vec<f32> = Query::<&Pos>::new().iter(&w).map(|p| p.0).collect();
    seen.sort_by(f32::total_cmp);
    let expected: Vec<f32> = (0..10_000).map(|i| i as f32 + 1.0).collect();
    assert_eq!(seen, expected);
}

#[test]
fn works_across_several_archetypes_and_honours_filters() {
    let mut w = world(100);
    for i in 0..50 {
        w.spawn_bundle((Pos(i as f32), Vel(1.0), Frozen));
    }
    Query::<&mut Pos, Without<Frozen>>::new().par_for_each_mut(&mut w, |p| p.0 = -1.0);
    let moved = Query::<&Pos>::new()
        .iter(&w)
        .filter(|p| p.0 == -1.0)
        .count();
    assert_eq!(moved, 100, "only entities without Frozen are touched");
    let frozen_untouched = Query::<&Pos, With<Frozen>>::new()
        .iter(&w)
        .all(|p| p.0 >= 0.0);
    assert!(frozen_untouched);
}

#[test]
fn mutated_rows_are_stamped_as_changed() {
    let mut w = world(100);
    w.begin_frame();
    assert_eq!(
        Query::<Entity, Changed<Pos>>::new().iter(&w).count(),
        0,
        "nothing changed since the frame began"
    );
    Query::<&mut Pos>::new().par_for_each_mut(&mut w, |p| p.0 += 1.0);
    assert_eq!(Query::<Entity, Changed<Pos>>::new().iter(&w).count(), 100);
}

#[test]
fn matches_the_sequential_iterator() {
    let mut a = world(3_000);
    let mut b = world(3_000);
    Query::<&mut Pos>::new().par_for_each_mut(&mut a, |p| p.0 = p.0 * 2.0 + 1.0);
    for p in Query::<&mut Pos>::new().iter_mut(&mut b) {
        p.0 = p.0 * 2.0 + 1.0;
    }
    let collect = |w: &World| {
        let mut v: Vec<f32> = Query::<&Pos>::new().iter(w).map(|p| p.0).collect();
        v.sort_by(f32::total_cmp);
        v
    };
    assert_eq!(collect(&a), collect(&b));
}

#[test]
#[should_panic(expected = "aliasing")]
fn aliasing_queries_are_rejected() {
    let mut w = world(4);
    Query::<(&mut Pos, &Pos)>::new().par_for_each_mut(&mut w, |_| {});
}

#[test]
fn empty_world_is_fine() {
    let mut w = World::new();
    Query::<&mut Pos>::new().par_for_each_mut(&mut w, |_| unreachable!());
}

#[test]
fn with_commands_applies_after_the_closure() {
    let mut w = world(3);
    let doomed: Vec<Entity> = Query::<Entity>::new().iter(&w).collect();
    let spawned = w.with_commands(|world, cmds| {
        for e in &doomed {
            if world.get_component::<Pos>(*e).is_some() {
                cmds.despawn(*e);
            }
        }
        cmds.spawn((Pos(99.0), Vel(0.0)))
    });
    assert_eq!(Query::<&Pos>::new().iter(&w).count(), 1);
    assert_eq!(w.get_component::<Pos>(spawned), Some(&Pos(99.0)));
}
