//! ECS micro-benchmarks: `cargo bench -p redox_ecs`.

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use redox_ecs::{ParallelQuery, Query, World};
use std::hint::black_box;

#[derive(Clone, Copy)]
struct Position(f32, f32, f32);
#[derive(Clone, Copy)]
struct Velocity(f32, f32, f32);

const N: usize = 100_000;

fn populated() -> World {
    let mut world = World::new();
    for i in 0..N {
        world.spawn_bundle((Position(i as f32, 0.0, 0.0), Velocity(1.0, 0.0, 0.0)));
    }
    world
}

fn bench_spawn(c: &mut Criterion) {
    c.bench_function("spawn_bundle_100k", |b| b.iter(|| black_box(populated())));
}

fn bench_iterate(c: &mut Criterion) {
    let world = populated();
    let query = Query::<(&Position, &Velocity)>::new();
    c.bench_function("iter_100k", |b| {
        b.iter(|| {
            let mut sum = 0.0;
            for (p, v) in query.iter(&world) {
                sum += p.0 + v.0;
            }
            black_box(sum)
        })
    });
    c.bench_function("par_iter_100k", |b| {
        use rayon::iter::ParallelIterator;
        b.iter(|| {
            let sum: f32 = query.par_iter(&world).map(|(p, v)| p.0 + v.0).sum();
            black_box(sum)
        })
    });
}

fn bench_migration(c: &mut Criterion) {
    c.bench_function("add_component_10k", |b| {
        b.iter_batched(
            || {
                let mut world = World::new();
                let entities: Vec<_> = (0..10_000)
                    .map(|i| world.spawn_bundle((Position(i as f32, 0.0, 0.0),)))
                    .collect();
                (world, entities)
            },
            |(mut world, entities)| {
                for e in entities {
                    world.add_component(e, Velocity(0.0, 1.0, 0.0));
                }
                black_box(world)
            },
            BatchSize::LargeInput,
        )
    });
}

criterion_group!(benches, bench_spawn, bench_iterate, bench_migration);
criterion_main!(benches);
