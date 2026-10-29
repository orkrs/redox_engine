//! Path requests answered in batches: at most a fixed number of searches per frame.

use std::collections::{HashMap, VecDeque};

use redox_ecs::Entity;
use redox_math::Vec3;

use crate::world::{AgentClass, NavWorld};

/// Identifier of a submitted path request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PathTicket(pub(crate) u64);

#[derive(Clone, Copy, Debug)]
struct PathRequest {
    ticket: PathTicket,
    from: Vec3,
    to: Vec3,
    class: AgentClass,
}

/// First-in first-out queue of path requests.
///
/// Requests are answered in submission order. [`PathQueue::run`] does at most `budget` searches,
/// so a burst of requests is spread over several frames.
#[derive(Debug, Default)]
pub struct PathQueue {
    pending: VecDeque<PathRequest>,
    owners: HashMap<PathTicket, (Entity, Vec3)>,
    next_ticket: u64,
}

impl PathQueue {
    /// An empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues a request for `entity` from `from` to `to`; `to` is remembered as the goal.
    pub fn submit(
        &mut self,
        entity: Entity,
        from: Vec3,
        to: Vec3,
        class: AgentClass,
    ) -> PathTicket {
        let ticket = PathTicket(self.next_ticket);
        self.next_ticket = self.next_ticket.wrapping_add(1);
        self.owners.insert(ticket, (entity, to));
        self.pending.push_back(PathRequest {
            ticket,
            from,
            to,
            class,
        });
        ticket
    }

    /// Answers up to `budget` requests in submission order. `None` is a failed search.
    pub fn run(&mut self, world: &NavWorld, budget: usize) -> Vec<(PathTicket, Option<Vec<Vec3>>)> {
        let mut results = Vec::with_capacity(budget.min(self.pending.len()));
        for _ in 0..budget {
            let Some(request) = self.pending.pop_front() else {
                break;
            };
            let path = world.find_path(request.from, request.to, request.class);
            results.push((request.ticket, path));
        }
        results
    }

    /// Entity and goal of a ticket, removed from the queue's bookkeeping.
    pub fn take_owner(&mut self, ticket: PathTicket) -> Option<(Entity, Vec3)> {
        self.owners.remove(&ticket)
    }

    /// Whether `entity` has a request that is not yet taken.
    pub fn is_pending(&self, entity: Entity) -> bool {
        self.owners.values().any(|(owner, _)| *owner == entity)
    }

    /// Number of requests not yet answered.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether no request waits for an answer.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}
