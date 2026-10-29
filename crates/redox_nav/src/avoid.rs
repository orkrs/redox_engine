//! Local avoidance: agents steer around each other (ORCA, inside landmass).
//!
//! The backend keeps one set of avoidance settings per archipelago. [`NavWorld::add_agent`]
//! applies the settings it is given, so the last agent added decides them; `max_neighbours`
//! has no backend counterpart and is ignored.

use redox_math::Vec3;

use crate::backend::{self, AgentHandle};
use crate::error::NavError;
use crate::world::NavWorld;

/// Avoidance settings. Units: metres and seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AvoidanceParams {
    /// Distance within which other agents are considered for avoidance.
    pub neighbour_distance: f32,
    /// Accepted by the API; not used by the backend.
    pub max_neighbours: u32,
    /// Time horizon for avoiding other agents.
    pub time_horizon: f32,
    /// Time horizon for avoiding obstacles (the mesh boundary).
    pub obstacle_time_horizon: f32,
}

impl Default for AvoidanceParams {
    fn default() -> Self {
        Self {
            neighbour_distance: 5.0,
            max_neighbours: 10,
            time_horizon: 2.0,
            obstacle_time_horizon: 1.0,
        }
    }
}

/// Handle of an agent added with [`NavWorld::add_agent`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NavAgentId(pub(crate) AgentHandle);

/// What an agent is doing, as the navigation backend reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentStatus {
    /// No target, or paused.
    Idle,
    /// Walking towards the target.
    Moving,
    /// Arrived at the target.
    Reached,
    /// Has a target but no path to it.
    NoPath,
    /// The agent or the target is not on the mesh.
    OffMesh,
}

impl NavWorld {
    /// Adds an agent at `position` and applies `params` to the avoidance of the world.
    ///
    /// # Errors
    ///
    /// [`NavError::BadAgent`] when `radius` or `max_speed` is not positive and finite.
    pub fn add_agent(
        &mut self,
        position: Vec3,
        radius: f32,
        max_speed: f32,
        params: AvoidanceParams,
    ) -> Result<NavAgentId, NavError> {
        if !(radius.is_finite() && radius > 0.0 && max_speed.is_finite() && max_speed > 0.0) {
            return Err(NavError::BadAgent);
        }
        let options = &mut self.archipelago.archipelago_options;
        options.neighbourhood = params.neighbour_distance;
        options.avoidance_time_horizon = params.time_horizon;
        options.obstacle_avoidance_time_horizon = params.obstacle_time_horizon;
        let handle = self
            .archipelago
            .add_agent(backend::agent(position, radius, max_speed));
        Ok(NavAgentId(handle))
    }

    /// Removes an agent; returns whether it existed.
    pub fn remove_agent(&mut self, id: NavAgentId) -> bool {
        if self.archipelago.get_agent(id.0).is_none() {
            return false;
        }
        self.archipelago.remove_agent(id.0);
        true
    }

    /// Sets where the agent wants to go; `None` makes it stand still (it still avoids others).
    pub fn set_goal(&mut self, id: NavAgentId, goal: Option<Vec3>) {
        if let Some(agent) = self.archipelago.get_agent_mut(id.0) {
            agent.current_target = goal.map(backend::to_lm);
        }
    }

    /// Status of an agent, or `None` for an unknown id.
    pub fn agent_status(&self, id: NavAgentId) -> Option<AgentStatus> {
        self.archipelago.get_agent(id.0).map(backend::status_of)
    }

    /// Number of agents in the world.
    pub fn agent_count(&self) -> usize {
        self.archipelago.get_agent_ids().len()
    }

    /// Gives the backend the current position and velocity of each agent, advances the
    /// avoidance by `dt` seconds and returns the desired velocity of each listed agent.
    ///
    /// Agents not listed keep their last state. Unknown ids are left out of the result.
    pub fn update_agents(
        &mut self,
        dt: f32,
        poses: &[(NavAgentId, Vec3, Vec3)],
    ) -> Vec<(NavAgentId, Vec3)> {
        for &(id, position, velocity) in poses {
            if let Some(agent) = self.archipelago.get_agent_mut(id.0) {
                agent.position = backend::to_lm(position);
                agent.velocity = backend::to_lm(velocity);
            }
        }
        self.archipelago.update(dt);
        poses
            .iter()
            .filter_map(|&(id, _, _)| {
                self.archipelago
                    .get_agent(id.0)
                    .map(|agent| (id, backend::from_lm(*agent.get_desired_velocity())))
            })
            .collect()
    }
}
