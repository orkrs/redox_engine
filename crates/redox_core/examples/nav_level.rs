//! Minimal navigation sample: loads a baked `.nav` beside the binary and walks one corridor.

#![allow(clippy::unwrap_used)]

use redox_core::config::EngineConfig;
use redox_core::{insert_nav_from_path, App};
use redox_ecs::{Entity, World};
use redox_math::{Transform, Vec3};
use redox_nav::{NavAgent, NavTarget};

fn corridor_nav_bytes() -> Vec<u8> {
    use redox_nav::{AgentParams, NavAreaId, NavData, NavPolygon, NavTile};
    let cols = 8_u16;
    let mut vertices = Vec::new();
    for row in 0..=1_u16 {
        for col in 0..=cols {
            vertices.push([f32::from(col), 0.0, f32::from(row)]);
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
    .to_bytes()
}

fn spawn_agent(world: &mut World) -> Entity {
    let entity = world.spawn();
    world.add_component(
        entity,
        NavAgent {
            class: redox_nav::AgentClass(0),
            radius: 0.4,
            height: 1.8,
            speed: 2.0,
            accel: 40.0,
        },
    );
    world.add_component(entity, Transform::from_translation(Vec3::new(0.5, 0.0, 0.5)));
    world.add_component(entity, NavTarget(Vec3::new(7.5, 0.0, 0.5)));
    entity
}

fn main() {
    let nav_path =
        std::env::temp_dir().join(format!("redox_nav_level_{}.nav", std::process::id()));
    std::fs::write(&nav_path, corridor_nav_bytes()).unwrap();
    App::new()
        .with_navigation()
        .run_with_setup(move |world| {
            insert_nav_from_path(world, &nav_path);
            spawn_agent(world);
        });
    let _ = std::fs::remove_file(nav_path);
}