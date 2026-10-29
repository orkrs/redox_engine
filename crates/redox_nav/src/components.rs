//! ECS components and events of agents that follow navmesh paths.

use redox_ecs::Entity;
use redox_math::Vec3;

use crate::avoid::NavAgentId;
use crate::world::AgentClass;

/// A walking agent. Its position is the entity's [`redox_math::Transform`] translation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavAgent {
    /// Class of the agent; decides which links it may use.
    pub class: AgentClass,
    /// Radius in metres (informational: the mesh is baked for the world's agent).
    pub radius: f32,
    /// Height in metres (informational).
    pub height: f32,
    /// Speed while walking, metres per second.
    pub speed: f32,
    /// Acceleration towards the walking speed, metres per second squared.
    pub accel: f32,
}

/// Where the agent should go. Removed when the agent arrives or the path fails.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavTarget(pub Vec3);

/// The planned path of an agent.
#[derive(Clone, Debug, PartialEq)]
pub struct NavPath {
    /// Waypoints from the agent to the goal.
    pub points: Vec<Vec3>,
    /// Index of the waypoint being approached.
    pub next: usize,
    /// The target this path leads to.
    pub goal: Vec3,
}

/// Current walking velocity of an agent, kept by [`crate::nav_follow_system`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NavVelocity(pub Vec3);

/// Sent when an agent reached the last waypoint of its path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathReached {
    /// The agent.
    pub entity: Entity,
}

/// Sent when no path to the target exists. The target is removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathFailed {
    /// The agent.
    pub entity: Entity,
}

/// Link between an entity and its agent in the avoidance world. Added by [`crate::nav_avoid_system`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NavHandle(pub NavAgentId);

/// Velocity the avoidance asks for this frame (world space, metres per second).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NavDesiredVelocity(pub Vec3);

/// Position of an agent with a character controller at the previous frame, to measure its velocity.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct NavPrevious(pub(crate) Vec3);
