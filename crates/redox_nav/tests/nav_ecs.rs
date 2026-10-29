//! Navigation in the ECS (NAV-03): path requests, following, arrival and failure events.

#![allow(clippy::unwrap_used)]

use redox_ecs::{Entity, Events, World};
use redox_math::{Transform, Vec3};
use redox_nav::{
    AgentClass, AgentParams, NavAgent, NavAreaId, NavData, NavPath, NavPolygon, NavTarget, NavTile,
    NavWorld, PathFailed, PathQueue, PathReached, nav_follow_system, nav_path_system,
};

/// One tile of `cols` x 2 cells (1 m each) along +X.
fn corridor(cols: u16) -> NavData {
    let mut vertices = Vec::new();
    for r in 0..=1u16 {
        for c in 0..=cols {
            vertices.push([f32::from(c), 0.0, f32::from(r)]);
        }
    }
    let stride = cols + 1;
    let polygons = (0..cols)
        .map(|c| NavPolygon {
            indices: vec![c, stride + c, stride + c + 1, c + 1],
            area: NavAreaId::WALKABLE,
        })
        .collect();
    NavData {
        agent: AgentParams::default(),
        tile_size: 16.0,
        tiles: vec![NavTile {
            coord: (0, 0),
            vertices,
            polygons,
        }],
    }
}

fn world(cols: u16) -> World {
    let mut world = World::new();
    world.insert_resource(NavWorld::from_data(&corridor(cols)).unwrap());
    world.insert_resource(PathQueue::new());
    world.insert_resource(Events::<PathReached>::new());
    world.insert_resource(Events::<PathFailed>::new());
    world
}

fn agent(world: &mut World, at: Vec3, target: Option<Vec3>) -> Entity {
    let e = world.spawn();
    world.add_component(
        e,
        NavAgent {
            class: AgentClass(0),
            radius: 0.4,
            height: 1.8,
            speed: 3.0,
            accel: 50.0,
        },
    );
    world.add_component(e, Transform::from_translation(at));
    if let Some(t) = target {
        world.add_component(e, NavTarget(t));
    }
    e
}

fn position(world: &World, e: Entity) -> Vec3 {
    world.get_component::<Transform>(e).unwrap().translation
}

#[test]
fn an_agent_with_a_target_gets_a_path() {
    let mut world = world(8);
    let e = agent(
        &mut world,
        Vec3::new(0.5, 0.0, 0.5),
        Some(Vec3::new(7.5, 0.0, 0.5)),
    );
    nav_path_system(&mut world);
    let path = world
        .get_component::<NavPath>(e)
        .expect("a path was planned");
    assert!(path.points.len() >= 2);
    assert_eq!(path.next, 0);
}

#[test]
fn an_agent_without_a_route_gets_path_failed() {
    let mut world = world(8);
    // The target is far away from the mesh.
    let e = agent(
        &mut world,
        Vec3::new(0.5, 0.0, 0.5),
        Some(Vec3::new(500.0, 0.0, 500.0)),
    );
    nav_path_system(&mut world);
    assert!(
        world.get_component::<NavTarget>(e).is_none(),
        "target removed"
    );
    assert!(world.get_component::<NavPath>(e).is_none());
    let events = world.get_resource::<Events<PathFailed>>().unwrap();
    assert!(events.iter().any(|ev| ev.entity == e));
}

#[test]
fn an_agent_reaches_its_target_and_the_target_is_removed() {
    let mut world = world(8);
    let goal = Vec3::new(7.5, 0.0, 0.5);
    let e = agent(&mut world, Vec3::new(0.5, 0.0, 0.5), Some(goal));
    for _ in 0..600 {
        nav_path_system(&mut world);
        nav_follow_system(&mut world, 1.0 / 60.0);
        if world.get_component::<NavTarget>(e).is_none() {
            break;
        }
    }
    assert!(world.get_component::<NavTarget>(e).is_none());
    assert!(
        position(&world, e).distance(goal) < 0.2,
        "{:?}",
        position(&world, e)
    );
    let events = world.get_resource::<Events<PathReached>>().unwrap();
    assert_eq!(events.iter().filter(|ev| ev.entity == e).count(), 1);
}

#[test]
fn the_agent_stays_on_the_mesh_while_following() {
    let mut world = world(8);
    let e = agent(
        &mut world,
        Vec3::new(0.5, 0.0, 0.5),
        Some(Vec3::new(7.5, 0.0, 1.5)),
    );
    for tick in 0..400 {
        nav_path_system(&mut world);
        nav_follow_system(&mut world, 1.0 / 60.0);
        if tick % 10 == 0 {
            let nav = world.get_resource::<NavWorld>().unwrap();
            assert!(
                nav.nearest_point(position(&world, e), 0.05).is_some(),
                "left the mesh at {:?}",
                position(&world, e)
            );
        }
    }
}

#[test]
fn systems_do_nothing_without_the_resources() {
    let mut world = World::new();
    let e = agent(&mut world, Vec3::ZERO, Some(Vec3::X));
    nav_path_system(&mut world);
    nav_follow_system(&mut world, 0.016);
    assert!(world.get_component::<NavTarget>(e).is_some());
}
