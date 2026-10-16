// Compile-fail: two live mutable iterators over the same World must be
// rejected — `iter_mut` requires an exclusive borrow of the world.
use redox_ecs::{Query, World};

#[derive(Debug, Clone)]
struct Health(f32);

fn main() {
    let mut world = World::new();
    let e = world.spawn();
    world.add_component(e, Health(100.0));

    let q = Query::<&mut Health>::new();
    let it1 = q.iter_mut(&mut world);
    let it2 = q.iter_mut(&mut world); //~ ERROR E0499: cannot borrow `world` as mutable more than once at a time

    let a = it1.next().unwrap();
    let b = it2.next().unwrap();
    a.0 -= 5.0;
    b.0 -= 5.0; // would alias if this compiled
}
