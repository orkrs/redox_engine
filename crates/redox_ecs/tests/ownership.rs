//! Ownership and drop-safety regression tests for the ECS storage.

use redox_ecs::World;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Tracked(Arc<AtomicUsize>);

impl Drop for Tracked {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct Other(u32);

#[test]
fn remove_component_drops_the_removed_value_exactly_once() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut world = World::new();
    let e = world.spawn();
    world.add_component(e, Tracked(drops.clone()));
    world.add_component(e, Other(7));

    assert!(world.remove_component::<Tracked>(e));
    assert_eq!(
        drops.load(Ordering::SeqCst),
        1,
        "removed value must be dropped"
    );
    assert_eq!(world.get_component::<Other>(e).map(|o| o.0), Some(7));

    world.despawn(e);
    assert_eq!(drops.load(Ordering::SeqCst), 1, "no double drop on despawn");
}

#[test]
fn removing_the_last_component_drops_it_once() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut world = World::new();
    let e = world.spawn();
    world.add_component(e, Tracked(drops.clone()));
    assert!(world.remove_component::<Tracked>(e));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn overwriting_a_component_drops_the_old_value() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut world = World::new();
    let e = world.spawn();
    world.add_component(e, Tracked(drops.clone()));
    world.add_component(e, Tracked(drops.clone()));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    drop(world);
    assert_eq!(
        drops.load(Ordering::SeqCst),
        2,
        "world drop releases the rest"
    );
}

#[test]
fn panicking_destructor_does_not_double_drop() {
    struct Bomb(Arc<AtomicUsize>, bool);
    impl Drop for Bomb {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
            if self.1 && !std::thread::panicking() {
                panic!("boom");
            }
        }
    }

    let drops = Arc::new(AtomicUsize::new(0));
    let mut world = World::new();
    let e = world.spawn();
    world.add_component(e, Bomb(drops.clone(), true));

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        world.add_component(e, Bomb(drops.clone(), false));
    }));
    assert!(result.is_err(), "the old value's destructor panics");
    assert_eq!(drops.load(Ordering::SeqCst), 1);

    // The slot must now hold the new (non-panicking) value, so tearing the world
    // down drops it exactly once more instead of re-dropping the old bomb.
    drop(world);
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}
