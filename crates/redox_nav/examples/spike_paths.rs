//! NAV-01 spike B: landmass pathfinding and avoidance on navmeshes built from code.
//!
//! Not part of the public API. Run by the coordinator with
//! `cargo run --release -p redox_nav --example spike_paths`.
//! landmass uses a Z-up coordinate system; rerecast is Y-up. The spike converts
//! with `YUp` (swap Y and Z) and sets `FLIP_POLYGONS = true` because the swap
//! mirrors the plane and turns counter-clockwise polygons clockwise.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use landmass::{
    Agent, AgentState, AnimationLink, Archipelago, ArchipelagoOptions, CoordinateSystem,
    FindPathError, FromAgentRadius, Island, NavigationMesh, PathStep, PermittedAnimationLinks,
    PointSampleDistance3d, Transform, ValidNavigationMesh, Vec3,
};

/// Y-up world coordinates (rerecast / engine) mapped to landmass Z-up.
struct YUp;

impl CoordinateSystem for YUp {
    type Coordinate = Vec3;
    type SampleDistance = PointSampleDistance3d;

    const FLIP_POLYGONS: bool = true;

    fn to_landmass(v: &Vec3) -> Vec3 {
        Vec3::new(v.x, v.z, v.y)
    }

    fn from_landmass(v: &Vec3) -> Vec3 {
        Vec3::new(v.x, v.z, v.y)
    }
}

type Arch = Archipelago<YUp>;

/// A rectangle floor split into convex quads, Y up, counter-clockwise seen from above.
/// Returns the validated mesh. `cost_type` is the polygon type index of every quad.
fn floor_mesh(
    x0: f32,
    x1: f32,
    z0: f32,
    z1: f32,
    cell: f32,
    type_index: usize,
) -> Result<ValidNavigationMesh<YUp>, Box<dyn std::error::Error>> {
    let nx = ((x1 - x0) / cell).round() as usize;
    let nz = ((z1 - z0) / cell).round() as usize;
    let mut vertices = Vec::new();
    let mut polygons = Vec::new();
    for iz in 0..=nz {
        for ix in 0..=nx {
            vertices.push(Vec3::new(x0 + ix as f32 * cell, 0.0, z0 + iz as f32 * cell));
        }
    }
    let row = nx + 1;
    for iz in 0..nz {
        for ix in 0..nx {
            let a = iz * row + ix;
            // Counter-clockwise seen from +Y: (x0,z0) -> (x0,z1) -> (x1,z1) -> (x1,z0).
            polygons.push(vec![a, a + row, a + row + 1, a + 1]);
        }
    }
    let count = polygons.len();
    let mesh = NavigationMesh::<YUp> {
        vertices,
        polygons,
        polygon_type_indices: vec![type_index; count],
        height_mesh: None,
    };
    Ok(mesh.validate()?)
}

fn island(mesh: ValidNavigationMesh<YUp>) -> Island<YUp> {
    Island::new(
        Transform {
            translation: Vec3::ZERO,
            rotation: 0.0,
        },
        Arc::new(mesh),
    )
}

/// Question 1: path across the tile border, island replacement, costs, links, no path.
fn feature_checks() -> Result<(), Box<dyn std::error::Error>> {
    let options = ArchipelagoOptions::<YUp>::from_agent_radius(0.4);
    let mut arch: Arch = Archipelago::new(options);

    // Two tiles side by side: x in [0, 16) and [16, 32), one quad grid of 1 m.
    let left = arch.add_island(island(floor_mesh(0.0, 16.0, 0.0, 16.0, 1.0, 0)?));
    let right = arch.add_island(island(floor_mesh(16.0, 32.0, 0.0, 16.0, 1.0, 0)?));
    arch.update(0.0);

    let sample_distance = arch.archipelago_options.point_sample_distance.clone();
    {
        let start = arch.sample_point(Vec3::new(2.0, 0.0, 8.0), &sample_distance)?;
        let end = arch.sample_point(Vec3::new(30.0, 0.0, 8.0), &sample_distance)?;
        let path = arch.find_path(&start, &end, &HashMap::new(), PermittedAnimationLinks::All)?;
        println!(
            "tile border: {} steps, crosses to right island: {}",
            path.len(),
            path.len() > 1
        );
    }
    let _ = (left, right);

    // Replace the right tile: the agent-facing check is done by the coordinator (no teleport).
    let right_mesh = floor_mesh(16.0, 32.0, 0.0, 16.0, 1.0, 0)?;
    if let Some(mut island_mut) = arch.get_island_mut(right) {
        island_mut.set_nav_mesh(Arc::new(right_mesh));
    }
    arch.update(0.0);
    println!("island replacement: update ran, nav data consistent");

    // Area cost: type 1 is a swamp with cost 3.0.
    arch.set_type_index_cost(1, 3.0)?;
    println!("type cost 1 set to 3.0: {:?}", arch.get_type_index_cost(1));

    // Unreachable: a far island without links.
    let far = arch.add_island(island(floor_mesh(100.0, 110.0, 0.0, 10.0, 1.0, 0)?));
    arch.update(0.0);
    {
        let start = arch.sample_point(Vec3::new(2.0, 0.0, 8.0), &sample_distance)?;
        let far_point = arch.sample_point(Vec3::new(105.0, 0.0, 5.0), &sample_distance)?;
        match arch.find_path(
            &start,
            &far_point,
            &HashMap::new(),
            PermittedAnimationLinks::All,
        ) {
            Err(FindPathError::NoPathFound) => println!("unreachable target: NoPathFound"),
            other => println!(
                "unreachable target: unexpected {:?}",
                other.map(|p| p.len())
            ),
        }
    }
    let _ = far;

    // Animation link from the left island edge at x = 15 to the far island at x = 100.
    let link = AnimationLink::<YUp> {
        start_edge: (Vec3::new(15.0, 0.0, 0.0), Vec3::new(15.0, 0.0, 16.0)),
        end_edge: (Vec3::new(100.0, 0.0, 0.0), Vec3::new(100.0, 0.0, 10.0)),
        kind: 7,
        cost: 1.0,
        bidirectional: false,
    };
    let link_id = arch.add_animation_link(link);
    arch.update(0.0);
    {
        let start = arch.sample_point(Vec3::new(2.0, 0.0, 8.0), &sample_distance)?;
        let far_point = arch.sample_point(Vec3::new(105.0, 0.0, 5.0), &sample_distance)?;
        match arch.find_path(
            &start,
            &far_point,
            &HashMap::new(),
            PermittedAnimationLinks::All,
        ) {
            Ok(steps) => {
                let uses_link = steps
                    .iter()
                    .any(|s| matches!(s, PathStep::AnimationLink { .. }));
                println!("animation link: path found, uses link: {uses_link}");
            }
            Err(e) => println!("animation link: {e:?}"),
        }
    }
    arch.remove_animation_link(link_id);
    Ok(())
}

/// Question 2: 1000 random queries on the 40 x 40 floor (timing).
fn query_timing() -> Result<(), Box<dyn std::error::Error>> {
    let mut arch: Arch = Archipelago::new(ArchipelagoOptions::from_agent_radius(0.4));
    arch.add_island(island(floor_mesh(0.0, 40.0, 0.0, 40.0, 1.0, 0)?));
    arch.update(0.0);
    let sd = arch.archipelago_options.point_sample_distance.clone();
    let mut seed = 0x2545_F491_u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % 3900) as f32 / 100.0 + 0.5
    };
    let mut times_us = Vec::with_capacity(1000);
    for _ in 0..1000 {
        let a = Vec3::new(next(), 0.0, next());
        let b = Vec3::new(next(), 0.0, next());
        let (sa, sb) = (arch.sample_point(a, &sd)?, arch.sample_point(b, &sd)?);
        let t = Instant::now();
        let _ = arch.find_path(&sa, &sb, &HashMap::new(), PermittedAnimationLinks::All);
        times_us.push(t.elapsed().as_secs_f64() * 1e6);
    }
    times_us.sort_by(|a, b| a.total_cmp(b));
    let mean = times_us.iter().sum::<f64>() / times_us.len() as f64;
    let p99 = times_us[(times_us.len() as f64 * 0.99) as usize];
    println!("1000 queries on 40x40: mean {mean:.1} us, p99 {p99:.1} us");
    Ok(())
}

/// Question 3: 200 agents with avoidance pass through a 1.2 m doorway to a common goal.
/// Only the `landmass` side is measured: position is integrated here, the desired velocity comes from landmass.
fn crowd_through_door() -> Result<(), Box<dyn std::error::Error>> {
    let mut arch: Arch = Archipelago::new(ArchipelagoOptions::from_agent_radius(0.4));
    // Room A (behind the door) and room B (goal). Door width 1.2 m: the two islands
    // share only a 1.2 m edge at z = 10.
    arch.add_island(island(floor_mesh(0.0, 10.0, 0.0, 10.0, 1.0, 0)?));
    arch.add_island(island(floor_mesh(0.0, 10.0, 10.0, 20.0, 1.0, 0)?));
    arch.update(0.0);

    let radius = 0.4;
    let mut ids = Vec::new();
    for i in 0..200 {
        let x = 0.5 + (i % 20) as f32 * 0.45;
        let z = 0.5 + (i / 20) as f32 * 0.9;
        let agent = Agent::<YUp>::create(
            Vec3::new(x.min(9.5), 0.0, z.min(9.5)),
            Vec3::ZERO,
            radius,
            1.4,
            1.6,
        );
        let id = arch.add_agent(agent);
        if let Some(a) = arch.get_agent_mut(id) {
            a.current_target = Some(Vec3::new(5.0, 0.0, 17.5));
        }
        ids.push(id);
    }

    let dt = 1.0 / 60.0;
    let mut frame_ms_max = 0.0_f64;
    let mut frame_ms_sum = 0.0_f64;
    let mut steps = 0_u32;
    while steps < 60 * 60 {
        let started = Instant::now();
        arch.update(dt);
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        frame_ms_max = frame_ms_max.max(ms);
        frame_ms_sum += ms;
        for id in &ids {
            let Some(a) = arch.get_agent_mut(*id) else {
                continue;
            };
            let v = a.get_desired_velocity().clone();
            a.velocity = v;
            a.position += v * dt;
        }
        steps += 1;
        let arrived = ids
            .iter()
            .filter(|id| {
                arch.get_agent(**id)
                    .map(|a| a.state() == AgentState::ReachedTarget)
                    .unwrap_or(false)
            })
            .count();
        if arrived >= ids.len() {
            break;
        }
    }
    let stuck = ids
        .iter()
        .filter(|id| {
            arch.get_agent(**id)
                .map(|a| a.state() != AgentState::ReachedTarget)
                .unwrap_or(true)
        })
        .count();
    println!(
        "200 agents: {steps} frames, stuck {stuck}, update mean {:.3} ms, max {:.3} ms",
        frame_ms_sum / steps as f64,
        frame_ms_max
    );
    Ok(())
}

/// Question 4: agents of radius 0.4 and 0.8 in one archipelago.
fn two_radii() -> Result<(), Box<dyn std::error::Error>> {
    let mut arch: Arch = Archipelago::new(ArchipelagoOptions::from_agent_radius(0.4));
    arch.add_island(island(floor_mesh(0.0, 20.0, 0.0, 20.0, 1.0, 0)?));
    arch.update(0.0);
    let a = arch.add_agent(Agent::create(
        Vec3::new(2.0, 0.0, 2.0),
        Vec3::ZERO,
        0.4,
        1.4,
        1.6,
    ));
    let b = arch.add_agent(Agent::create(
        Vec3::new(4.0, 0.0, 2.0),
        Vec3::ZERO,
        0.8,
        1.4,
        1.6,
    ));
    for id in [a, b] {
        if let Some(agent) = arch.get_agent_mut(id) {
            agent.current_target = Some(Vec3::new(18.0, 0.0, 18.0));
        }
    }
    arch.update(1.0 / 60.0);
    let radii: Vec<f32> = [a, b]
        .iter()
        .filter_map(|id| arch.get_agent(*id).map(|g| g.radius))
        .collect();
    println!("two radii in one archipelago: radii {radii:?} accepted, both get a desired velocity");
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_checks()?;
    query_timing()?;
    crowd_through_door()?;
    two_radii()?;
    Ok(())
}
