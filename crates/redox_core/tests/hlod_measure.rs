//! VG-04 step 5: CPU cost and draw-call count of HLOD for a scene of 4000 small
//! objects in 40 cells, seen from about 300 m. Ignored by default; run with
//! `cargo test -p redox_core --test hlod_measure -- --ignored --nocapture`.
//! GPU frame timings (`FrameCpuTimings::total_ms`) are not measured here.

#![allow(clippy::unwrap_used)]

use std::time::Instant;

use redox_core::hlod_system::{HlodRuntime, hlod_frame};
use redox_ecs::{Entity, World};
use redox_math::{Aabb, Mat4, Vec3};
use redox_render::hlod::HlodConfig;
use redox_render::hlod_gpu::ProxyGpu;
use redox_render::shadow::ShadowClass;
use redox_render::systems::RenderObject;
use redox_scene::{ActorKindTable, PartitionCell, StreamedActor};

const KINDS: &str = r#"(kinds: { "prop": (mesh: "meshes/prop.glb", color: (0.5, 0.5, 0.5)) })"#;
const CELLS_X: i32 = 8;
const CELLS_Y: i32 = 5;
const PER_CELL: usize = 100;
const CELL_SIZE: f32 = 50.0;
const WARMUP_FRAMES: usize = 30;
const TIMED_FRAMES: usize = 100;

fn detail_object(id: u32) -> RenderObject {
    RenderObject {
        id,
        shadow_class: ShadowClass::Auto,
        deforms: false,
        pose_version: 0,
        model_matrix: Mat4::IDENTITY,
        color: [1.0; 4],
        mesh_index: 0,
        material_index: 0,
        bounds: Aabb::empty(),
    }
}

/// Spawns 40 cells of 100 actors and returns the entities.
fn spawn_scene(world: &mut World) -> Vec<Entity> {
    let mut entities = Vec::new();
    for x in 0..CELLS_X {
        for y in 0..CELLS_Y {
            let cell = PartitionCell { x, y };
            for _ in 0..PER_CELL {
                let id = format!("prop_{}", entities.len());
                let entity = world.spawn_bundle((StreamedActor {
                    cell,
                    id,
                    kind: "prop".into(),
                },));
                entities.push(entity);
            }
        }
    }
    entities
}

/// Average microseconds of `hlod_frame` over `frames` frames, and the object count of the last one.
fn time_frames(
    world: &mut World,
    entities: &[Entity],
    viewer: Vec3,
    dt: f32,
    frames: usize,
) -> (f64, usize) {
    let mut total = 0.0;
    let mut count = 0;
    for _ in 0..frames {
        let mut objects: Vec<RenderObject> =
            entities.iter().map(|e| detail_object(e.id())).collect();
        let start = Instant::now();
        hlod_frame(world, viewer, dt, &mut objects);
        total += start.elapsed().as_secs_f64() * 1e6;
        count = objects.len();
    }
    (total / frames as f64, count)
}

#[test]
#[ignore = "measurement: run with --ignored --nocapture"]
fn four_thousand_objects_in_forty_cells_seen_from_300m() {
    let mut world = World::new();
    world.insert_resource(ActorKindTable::from_ron(KINDS).unwrap());
    let entities = spawn_scene(&mut world);
    // Viewer at x = -275: the nearest cell centre is about 300 m away.
    let viewer = Vec3::new(-275.0, 0.0, 125.0);

    let (without_us, without_draws) = time_frames(&mut world, &entities, viewer, 0.0, TIMED_FRAMES);

    let cells: Vec<PartitionCell> = (0..CELLS_X)
        .flat_map(|x| (0..CELLS_Y).map(move |y| PartitionCell { x, y }))
        .collect();
    let mut runtime = HlodRuntime::new(
        HlodConfig::default(),
        CELL_SIZE,
        Vec3::ZERO,
        cells.iter().copied(),
    );
    for cell in &cells {
        runtime.register_proxy(
            *cell,
            ProxyGpu {
                mesh_slot: 0,
                material_slot: 0,
                bounds: Aabb::empty(),
            },
        );
    }
    world.insert_resource(runtime);
    let _ = time_frames(&mut world, &entities, viewer, 0.1, WARMUP_FRAMES);
    let (with_us, with_draws) = time_frames(&mut world, &entities, viewer, 0.0, TIMED_FRAMES);

    println!("HLOD measurement, 4000 objects in 40 cells at about 300 m");
    println!("  without HLOD: {without_draws} draw calls, hlod_frame {without_us:.1} us/frame");
    println!("  with HLOD:    {with_draws} draw calls, hlod_frame {with_us:.1} us/frame");
    assert_eq!(
        without_draws,
        CELLS_X as usize * CELLS_Y as usize * PER_CELL
    );
    assert_eq!(with_draws, CELLS_X as usize * CELLS_Y as usize);
}
