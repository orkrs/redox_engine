//! Per-frame systems: answering path requests, walking agents along their paths, and local
//! avoidance with movement.

use std::collections::HashMap;

use redox_ecs::{Entity, Events, World};
use redox_math::{Transform, Vec3};
use redox_physics::CharacterController;

use crate::avoid::{AgentStatus, AvoidanceParams, NavAgentId};
use crate::components::{
    NavAgent, NavDesiredVelocity, NavHandle, NavPath, NavPrevious, NavTarget, NavVelocity,
    PathFailed, PathReached,
};
use crate::queue::PathQueue;
use crate::world::NavWorld;

/// Path searches answered per frame.
pub const PATH_QUERIES_PER_FRAME: usize = 16;
/// Distance at which a waypoint counts as reached, in metres.
pub const ARRIVE_DISTANCE: f32 = 0.1;
/// Largest distance between an agent and the mesh for the height to follow the mesh, in metres.
const GROUND_SNAP_DISTANCE: f32 = 0.5;
/// Share of the height gap closed per frame when following the mesh.
const GROUND_SNAP_RATE: f32 = 0.5;

/// Requests paths for agents with a target and no current path, then stores the answers.
///
/// Needs the [`NavWorld`] and [`PathQueue`] resources; does nothing without them.
/// A failed search sends [`PathFailed`] and removes the target.
pub fn nav_path_system(world: &mut World) {
    if world.get_resource::<NavWorld>().is_none() || world.get_resource::<PathQueue>().is_none() {
        return;
    }
    let (Some(nav), Some(mut queue)) = (
        world.remove_resource::<NavWorld>(),
        world.remove_resource::<PathQueue>(),
    ) else {
        return;
    };
    submit_requests(world, &mut queue);
    let mut failed = Vec::new();
    for (ticket, path) in queue.run(&nav, PATH_QUERIES_PER_FRAME) {
        let Some((entity, goal)) = queue.take_owner(ticket) else {
            continue;
        };
        if !is_current_target(world, entity, goal) {
            continue;
        }
        match path {
            Some(points) => world.add_component(
                entity,
                NavPath {
                    points,
                    next: 0,
                    goal,
                },
            ),
            None => {
                world.remove_component::<NavTarget>(entity);
                world.remove_component::<NavPath>(entity);
                failed.push(entity);
            }
        }
    }
    world.insert_resource(nav);
    world.insert_resource(queue);
    send_events::<PathFailed>(world, failed, |entity| PathFailed { entity });
}

/// Queues a request for every agent whose target has no path yet.
fn submit_requests(world: &World, queue: &mut PathQueue) {
    let agents: Vec<(Entity, NavAgent, Vec3, Vec3)> = world
        .query::<(Entity, &NavAgent, &NavTarget, &Transform)>()
        .iter(world)
        .map(|(entity, agent, target, transform)| (entity, *agent, target.0, transform.translation))
        .collect();
    for (entity, agent, goal, position) in agents {
        let has_path = world
            .get_component::<NavPath>(entity)
            .is_some_and(|path| path.goal == goal);
        if has_path || queue.is_pending(entity) {
            continue;
        }
        queue.submit(entity, position, goal, agent.class);
    }
}

/// True while the entity still wants to reach `goal`.
fn is_current_target(world: &World, entity: Entity, goal: Vec3) -> bool {
    world.get_component::<NavAgent>(entity).is_some()
        && world
            .get_component::<NavTarget>(entity)
            .is_some_and(|target| target.0 == goal)
}

/// Walks agents along their paths.
///
/// Each agent moves towards its next waypoint with its speed and acceleration. A waypoint
/// within [`ARRIVE_DISTANCE`] is passed; after the last one the agent sends [`PathReached`],
/// and its target and path are removed. Agents with a path that does not lead to their target
/// wait for a new path.
pub fn nav_follow_system(world: &mut World, dt: f32) {
    let movers: Vec<(Entity, NavAgent, NavPath, Vec3)> = world
        .query::<(Entity, &NavAgent, &NavPath, &Transform)>()
        .iter(world)
        .map(|(entity, agent, path, transform)| {
            (entity, *agent, path.clone(), transform.translation)
        })
        .collect();
    let mut reached = Vec::new();
    for (entity, agent, path, position) in movers {
        if !is_current_target(world, entity, path.goal) {
            continue;
        }
        let velocity = world
            .get_component::<NavVelocity>(entity)
            .map(|v| v.0)
            .unwrap_or(Vec3::ZERO);
        match advance(&agent, path, position, velocity, dt) {
            Advance::Moved {
                position,
                velocity,
                path,
            } => {
                if let Some(transform) = world.get_component_mut::<Transform>(entity) {
                    transform.translation = position;
                }
                store_velocity(world, entity, velocity);
                if let Some(slot) = world.get_component_mut::<NavPath>(entity) {
                    *slot = path;
                }
            }
            Advance::Reached => {
                world.remove_component::<NavPath>(entity);
                world.remove_component::<NavTarget>(entity);
                reached.push(entity);
            }
        }
    }
    send_events::<PathReached>(world, reached, |entity| PathReached { entity });
}

/// Result of one movement step.
enum Advance {
    Moved {
        position: Vec3,
        velocity: Vec3,
        path: NavPath,
    },
    Reached,
}

/// One step of movement towards the next waypoint, passing waypoints already reached.
fn advance(
    agent: &NavAgent,
    mut path: NavPath,
    position: Vec3,
    velocity: Vec3,
    dt: f32,
) -> Advance {
    while path.next < path.points.len()
        && position.distance(path.points[path.next]) <= ARRIVE_DISTANCE
    {
        path.next += 1;
    }
    let Some(&waypoint) = path.points.get(path.next) else {
        return Advance::Reached;
    };
    let to_waypoint = waypoint - position;
    let distance = to_waypoint.length();
    let desired = if distance > f32::EPSILON {
        to_waypoint / distance * agent.speed
    } else {
        Vec3::ZERO
    };
    let new_velocity = velocity + (desired - velocity).clamp_length_max(agent.accel.max(0.0) * dt);
    let step = new_velocity * dt;
    let new_position = if step.length() >= distance {
        waypoint
    } else {
        position + step
    };
    Advance::Moved {
        position: new_position,
        velocity: new_velocity,
        path,
    }
}

/// Local avoidance for every agent: adds new agents to the [`NavWorld`], sends their targets as
/// goals, asks the backend for desired velocities ([`NavDesiredVelocity`]) and reports arrivals.
///
/// Used instead of [`nav_path_system`] and [`nav_follow_system`] when the world has agents.
/// An agent whose target is reached sends [`PathReached`]; one with no path or off the mesh
/// sends [`PathFailed`]. Both lose their target.
pub fn nav_avoid_system(world: &mut World, dt: f32) {
    if world.get_resource::<NavWorld>().is_none() {
        return;
    }
    let Some(mut nav) = world.remove_resource::<NavWorld>() else {
        return;
    };
    add_missing_agents(world, &mut nav);
    remove_orphan_agents(world, &mut nav);
    let entries = avoidance_entries(world);
    for (entity, id, _) in &entries {
        let goal = world
            .get_component::<NavTarget>(*entity)
            .map(|target| target.0);
        nav.set_goal(*id, goal);
    }
    let poses: Vec<(NavAgentId, Vec3, Vec3)> = entries
        .iter()
        .map(|(_, id, (position, velocity))| (*id, *position, *velocity))
        .collect();
    let desired: HashMap<NavAgentId, Vec3> = nav.update_agents(dt, &poses).into_iter().collect();
    let mut failed = Vec::new();
    let mut reached = Vec::new();
    for (entity, id, _) in &entries {
        if let Some(velocity) = desired.get(id) {
            store_desired(world, *entity, *velocity);
        }
        if world.get_component::<NavTarget>(*entity).is_none() {
            continue;
        }
        match nav.agent_status(*id) {
            Some(AgentStatus::Reached) => reached.push(*entity),
            Some(AgentStatus::NoPath | AgentStatus::OffMesh) => failed.push(*entity),
            _ => {}
        }
    }
    for entity in reached.iter().chain(failed.iter()) {
        world.remove_component::<NavTarget>(*entity);
    }
    world.insert_resource(nav);
    send_events::<PathReached>(world, reached, |entity| PathReached { entity });
    send_events::<PathFailed>(world, failed, |entity| PathFailed { entity });
}

/// Creates an avoidance agent for every entity with [`NavAgent`] and a transform, but no handle.
fn add_missing_agents(world: &mut World, nav: &mut NavWorld) {
    let new_agents: Vec<(Entity, NavAgent, Vec3)> = world
        .query::<(Entity, &NavAgent, &Transform)>()
        .iter(world)
        .filter(|(entity, _, _)| world.get_component::<NavHandle>(*entity).is_none())
        .map(|(entity, agent, transform)| (entity, *agent, transform.translation))
        .collect();
    for (entity, agent, position) in new_agents {
        match nav.add_agent(
            position,
            agent.radius,
            agent.speed,
            AvoidanceParams::default(),
        ) {
            Ok(id) => world.add_component(entity, NavHandle(id)),
            Err(error) => log::warn!("agent {entity:?} is not in avoidance: {error}"),
        }
    }
}

/// Removes the avoidance agent of every entity whose [`NavAgent`] was removed.
fn remove_orphan_agents(world: &mut World, nav: &mut NavWorld) {
    let orphans: Vec<(Entity, NavHandle)> = world
        .query::<(Entity, &NavHandle)>()
        .iter(world)
        .filter(|(entity, _)| world.get_component::<NavAgent>(*entity).is_none())
        .map(|(entity, handle)| (entity, *handle))
        .collect();
    for (entity, handle) in orphans {
        nav.remove_agent(handle.0);
        world.remove_component::<NavHandle>(entity);
    }
}

/// Entity, avoidance id and (position, velocity) of every agent with a handle.
fn avoidance_entries(world: &World) -> Vec<(Entity, NavAgentId, (Vec3, Vec3))> {
    world
        .query::<(Entity, &NavHandle, &Transform)>()
        .iter(world)
        .map(|(entity, handle, transform)| {
            let velocity = world
                .get_component::<NavVelocity>(entity)
                .map(|v| v.0)
                .unwrap_or(Vec3::ZERO);
            (entity, handle.0, (transform.translation, velocity))
        })
        .collect()
}

/// Moves agents by their [`NavDesiredVelocity`].
///
/// An agent with a [`CharacterController`] gets the step as its `desired_translation`; the
/// physics moves it, and its velocity is measured from the position at the next frame. An agent
/// without one moves directly, only to a point that is still on the mesh, and its height is
/// pulled towards the mesh.
pub fn nav_move_system(world: &mut World, dt: f32) {
    for step in plan_moves(world, dt) {
        match step {
            Move::Controlled {
                entity,
                step,
                translation,
            } => {
                if let Some(controller) = world.get_component_mut::<CharacterController>(entity) {
                    controller.desired_translation = step;
                }
                let previous = world
                    .get_component::<NavPrevious>(entity)
                    .map(|p| p.0)
                    .unwrap_or(translation);
                let velocity = if dt > 0.0 {
                    (translation - previous) / dt
                } else {
                    Vec3::ZERO
                };
                store_velocity(world, entity, velocity);
                store_previous(world, entity, translation);
            }
            Move::Direct {
                entity,
                translation,
                velocity,
            } => {
                if let Some(transform) = world.get_component_mut::<Transform>(entity) {
                    transform.translation = translation;
                }
                store_velocity(world, entity, velocity);
            }
        }
    }
}

/// One agent's movement for this frame.
enum Move {
    Controlled {
        entity: Entity,
        step: Vec3,
        translation: Vec3,
    },
    Direct {
        entity: Entity,
        translation: Vec3,
        velocity: Vec3,
    },
}

/// Works out every agent's movement while only reading the world.
fn plan_moves(world: &World, dt: f32) -> Vec<Move> {
    let nav = world.get_resource::<NavWorld>();
    let desired: Vec<(Entity, Vec3)> = world
        .query::<(Entity, &NavDesiredVelocity)>()
        .iter(world)
        .map(|(entity, desired)| (entity, desired.0))
        .collect();
    let mut plan = Vec::new();
    for (entity, velocity) in desired {
        let Some(translation) = world
            .get_component::<Transform>(entity)
            .map(|t| t.translation)
        else {
            continue;
        };
        let step = velocity * dt;
        if world.get_component::<CharacterController>(entity).is_some() {
            plan.push(Move::Controlled {
                entity,
                step,
                translation,
            });
            continue;
        }
        let next = translation + step;
        let ground = match nav {
            Some(nav) => nav.nearest_point(next, GROUND_SNAP_DISTANCE),
            None => Some(next),
        };
        let Some(ground) = ground else {
            continue;
        };
        let height = next.y + (ground.y - next.y) * GROUND_SNAP_RATE;
        plan.push(Move::Direct {
            entity,
            translation: Vec3::new(next.x, height, next.z),
            velocity,
        });
    }
    plan
}

fn store_desired(world: &mut World, entity: Entity, velocity: Vec3) {
    if let Some(slot) = world.get_component_mut::<NavDesiredVelocity>(entity) {
        slot.0 = velocity;
    } else {
        world.add_component(entity, NavDesiredVelocity(velocity));
    }
}

fn store_velocity(world: &mut World, entity: Entity, velocity: Vec3) {
    if let Some(slot) = world.get_component_mut::<NavVelocity>(entity) {
        slot.0 = velocity;
    } else {
        world.add_component(entity, NavVelocity(velocity));
    }
}

fn store_previous(world: &mut World, entity: Entity, translation: Vec3) {
    if let Some(slot) = world.get_component_mut::<NavPrevious>(entity) {
        slot.0 = translation;
    } else {
        world.add_component(entity, NavPrevious(translation));
    }
}

/// Sends one event per entity, if the event resource exists.
fn send_events<T: Send + Sync + 'static>(
    world: &mut World,
    entities: Vec<Entity>,
    make: impl Fn(Entity) -> T,
) {
    if entities.is_empty() {
        return;
    }
    if let Some(events) = world.get_resource_mut::<Events<T>>() {
        for entity in entities {
            events.send(make(entity));
        }
    }
}
