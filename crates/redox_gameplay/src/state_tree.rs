//! Hierarchical StateTree — stateful AI / game-flow FSM.
//!
//! Evaluate transitions each frame, then tick the active leaf. Consumers drive
//! behaviour by matching [`StateTree::active_name`] (or id) after [`StateTree::tick`].
//!
//! # Example (sentinel patrol → chase)
//! ```ignore
//! use redox_gameplay::state_tree::{sentinel_patrol_chase, StateTreeContext};
//!
//! let mut tree = sentinel_patrol_chase();
//! let mut ctx = StateTreeContext::new();
//! ctx.distance_to_target = 12.0;
//! tree.tick(&mut ctx, 1.0 / 60.0);
//! assert_eq!(tree.active_name(), "Patrol");
//!
//! ctx.distance_to_target = 4.0; // within chase range
//! tree.tick(&mut ctx, 1.0 / 60.0);
//! assert_eq!(tree.active_name(), "Chase");
//! ```

use std::collections::HashMap;

/// Stable identity for a state node inside a [`StateTree`].
pub type StateId = u32;

/// Shared blackboard read by transition conditions and written by game code.
#[derive(Clone, Debug, Default)]
pub struct StateTreeContext {
    /// Distance from agent to its current interest target (player, waypoint, …).
    pub distance_to_target: f32,
    /// Seconds spent in the current active leaf since last transition.
    pub elapsed_in_state: f32,
    /// Named boolean flags (`"alert"`, `"has_line_of_sight"`, …).
    pub flags: HashMap<&'static str, bool>,
    /// Named float values for custom guards.
    pub floats: HashMap<&'static str, f32>,
}

impl StateTreeContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_flag(&mut self, name: &'static str, value: bool) {
        self.flags.insert(name, value);
    }

    pub fn flag(&self, name: &'static str) -> bool {
        self.flags.get(name).copied().unwrap_or(false)
    }

    pub fn set_float(&mut self, name: &'static str, value: f32) {
        self.floats.insert(name, value);
    }

    pub fn float(&self, name: &'static str) -> f32 {
        self.floats.get(name).copied().unwrap_or(0.0)
    }
}

/// Condition evaluated against a [`StateTreeContext`] when considering a transition.
#[derive(Clone, Debug)]
pub enum TransitionCondition {
    Always,
    Never,
    /// `distance_to_target <= threshold`
    TargetCloserThan(f32),
    /// `distance_to_target >= threshold`
    TargetFartherThan(f32),
    /// `elapsed_in_state >= secs`
    ElapsedGreaterThan(f32),
    FlagSet(&'static str),
    FlagClear(&'static str),
    FloatAtLeast(&'static str, f32),
    FloatAtMost(&'static str, f32),
    And(Box<TransitionCondition>, Box<TransitionCondition>),
    Or(Box<TransitionCondition>, Box<TransitionCondition>),
    Not(Box<TransitionCondition>),
}

impl TransitionCondition {
    pub fn evaluate(&self, ctx: &StateTreeContext) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::TargetCloserThan(t) => ctx.distance_to_target <= *t,
            Self::TargetFartherThan(t) => ctx.distance_to_target >= *t,
            Self::ElapsedGreaterThan(t) => ctx.elapsed_in_state >= *t,
            Self::FlagSet(name) => ctx.flag(name),
            Self::FlagClear(name) => !ctx.flag(name),
            Self::FloatAtLeast(name, v) => ctx.float(name) >= *v,
            Self::FloatAtMost(name, v) => ctx.float(name) <= *v,
            Self::And(a, b) => a.evaluate(ctx) && b.evaluate(ctx),
            Self::Or(a, b) => a.evaluate(ctx) || b.evaluate(ctx),
            Self::Not(inner) => !inner.evaluate(ctx),
        }
    }
}

/// Edge from the owning state to `target` when `condition` is true.
#[derive(Clone, Debug)]
pub struct StateTransition {
    pub target: StateId,
    pub condition: TransitionCondition,
}

impl StateTransition {
    pub fn new(target: StateId, condition: TransitionCondition) -> Self {
        Self { target, condition }
    }
}

/// Single node in the hierarchical tree.
#[derive(Clone, Debug)]
pub struct StateNode {
    pub id: StateId,
    pub name: String,
    pub parent: Option<StateId>,
    pub transitions: Vec<StateTransition>,
}

impl StateNode {
    pub fn new(id: StateId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            parent: None,
            transitions: Vec::new(),
        }
    }

    pub fn with_parent(mut self, parent: StateId) -> Self {
        self.parent = Some(parent);
        self
    }

    pub fn with_transition(mut self, transition: StateTransition) -> Self {
        self.transitions.push(transition);
        self
    }
}

/// Result of one [`StateTree::tick`] / [`StateTree::evaluate`] pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StateTickResult {
    pub active: StateId,
    pub transitioned: bool,
    pub exited: Option<StateId>,
    pub entered: Option<StateId>,
}

/// Hierarchical stateful AI / game-flow tree.
#[derive(Clone, Debug)]
pub struct StateTree {
    nodes: HashMap<StateId, StateNode>,
    active: StateId,
    root: StateId,
}

impl StateTree {
    /// Builds a tree from an explicit root and node list. Panics if `root` is missing
    /// or any transition target is unknown.
    pub fn new(root: StateId, nodes: Vec<StateNode>) -> Self {
        let map: HashMap<StateId, StateNode> = nodes.into_iter().map(|n| (n.id, n)).collect();
        assert!(map.contains_key(&root), "StateTree root id missing");
        for node in map.values() {
            for t in &node.transitions {
                assert!(
                    map.contains_key(&t.target),
                    "transition target {} missing",
                    t.target
                );
            }
            if let Some(p) = node.parent {
                assert!(map.contains_key(&p), "parent {} missing", p);
            }
        }
        Self {
            nodes: map,
            active: root,
            root,
        }
    }

    pub fn root(&self) -> StateId {
        self.root
    }

    pub fn active_id(&self) -> StateId {
        self.active
    }

    pub fn active_name(&self) -> &str {
        self.nodes
            .get(&self.active)
            .map(|n| n.name.as_str())
            .unwrap_or("<invalid>")
    }

    pub fn node(&self, id: StateId) -> Option<&StateNode> {
        self.nodes.get(&id)
    }

    /// All nodes (for editor debugger layout).
    pub fn iter_nodes(&self) -> impl Iterator<Item = &StateNode> + '_ {
        self.nodes.values()
    }

    /// Ancestry from `id` up to (and including) the root, leaf-first.
    pub fn ancestry(&self, id: StateId) -> Vec<StateId> {
        let mut chain = Vec::new();
        let mut cur = Some(id);
        while let Some(id) = cur {
            chain.push(id);
            cur = self.nodes.get(&id).and_then(|n| n.parent);
        }
        chain
    }

    /// Evaluates transitions from the active leaf, then parents (leaf wins).
    /// Does not advance `elapsed_in_state`. Returns whether a transition fired.
    pub fn evaluate(&mut self, ctx: &mut StateTreeContext) -> StateTickResult {
        let chain = self.ancestry(self.active);
        for id in chain {
            let Some(node) = self.nodes.get(&id) else {
                continue;
            };
            for transition in &node.transitions {
                if transition.condition.evaluate(ctx) {
                    let from = self.active;
                    let to = transition.target;
                    if from != to {
                        self.active = to;
                        ctx.elapsed_in_state = 0.0;
                        return StateTickResult {
                            active: to,
                            transitioned: true,
                            exited: Some(from),
                            entered: Some(to),
                        };
                    }
                }
            }
        }
        StateTickResult {
            active: self.active,
            transitioned: false,
            exited: None,
            entered: None,
        }
    }

    /// Evaluate transitions, then advance time in the (possibly new) active state.
    pub fn tick(&mut self, ctx: &mut StateTreeContext, delta_time: f32) -> StateTickResult {
        let result = self.evaluate(ctx);
        ctx.elapsed_in_state += delta_time.max(0.0);
        result
    }

    /// Force a transition ignoring guards. Resets `elapsed_in_state`.
    pub fn force_transition(&mut self, to: StateId, ctx: &mut StateTreeContext) -> bool {
        if !self.nodes.contains_key(&to) || self.active == to {
            return false;
        }
        self.active = to;
        ctx.elapsed_in_state = 0.0;
        true
    }
}

/// Fluent builder for [`StateTree`].
#[derive(Default)]
pub struct StateTreeBuilder {
    next_id: StateId,
    nodes: Vec<StateNode>,
    root: Option<StateId>,
}

impl StateTreeBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_state(&mut self, name: impl Into<String>) -> StateId {
        let id = self.next_id;
        self.next_id += 1;
        self.nodes.push(StateNode::new(id, name));
        if self.root.is_none() {
            self.root = Some(id);
        }
        id
    }

    pub fn set_root(&mut self, id: StateId) {
        self.root = Some(id);
    }

    pub fn set_parent(&mut self, child: StateId, parent: StateId) {
        if let Some(node) = self.nodes.iter_mut().find(|n| n.id == child) {
            node.parent = Some(parent);
        }
    }

    pub fn add_transition(
        &mut self,
        from: StateId,
        to: StateId,
        condition: TransitionCondition,
    ) {
        if let Some(node) = self.nodes.iter_mut().find(|n| n.id == from) {
            node.transitions.push(StateTransition::new(to, condition));
        }
    }

    pub fn build(self) -> StateTree {
        let root = self.root.expect("StateTreeBuilder requires at least one state");
        StateTree::new(root, self.nodes)
    }
}

/// Ready-made patrol ↔ chase tree for vault sentinels (or similar guards).
///
/// - **Patrol** → **Chase** when `distance_to_target <= chase_enter`
/// - **Chase** → **Patrol** when `distance_to_target >= chase_exit`
pub fn sentinel_patrol_chase() -> StateTree {
    sentinel_patrol_chase_ranges(6.0, 14.0)
}

/// Same as [`sentinel_patrol_chase`] with explicit enter/exit distances (hysteresis).
pub fn sentinel_patrol_chase_ranges(chase_enter: f32, chase_exit: f32) -> StateTree {
    let mut b = StateTreeBuilder::new();
    let root = b.add_state("Root");
    let patrol = b.add_state("Patrol");
    let chase = b.add_state("Chase");
    b.set_parent(patrol, root);
    b.set_parent(chase, root);
    b.add_transition(
        patrol,
        chase,
        TransitionCondition::TargetCloserThan(chase_enter),
    );
    b.add_transition(
        chase,
        patrol,
        TransitionCondition::TargetFartherThan(chase_exit),
    );
    // Start in Patrol (not Root) so active_name is useful immediately.
    let mut tree = b.build();
    let mut ctx = StateTreeContext::new();
    tree.force_transition(patrol, &mut ctx);
    let _ = root;
    tree
}

/// ECS component wrapping a tree + blackboard for AI agents / game phases.
#[derive(Clone, Debug)]
pub struct StateTreeComponent {
    pub tree: StateTree,
    pub ctx: StateTreeContext,
}

impl StateTreeComponent {
    pub fn new(tree: StateTree) -> Self {
        Self {
            tree,
            ctx: StateTreeContext::new(),
        }
    }

    pub fn patrol_chase() -> Self {
        Self::new(sentinel_patrol_chase())
    }

    /// Convenience: set target distance, then tick.
    pub fn tick_with_distance(&mut self, distance: f32, dt: f32) -> StateTickResult {
        self.ctx.distance_to_target = distance;
        self.tree.tick(&mut self.ctx, dt)
    }
}

/// Ticks every [`StateTreeComponent`] in the world (AI agents, phase machines).
pub fn state_tree_system(world: &mut redox_ecs::World, delta_time: f32) {
    use redox_ecs::Query;
    for agent in Query::<&mut StateTreeComponent>::new().iter_mut(world) {
        agent.tree.tick(&mut agent.ctx, delta_time);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patrol_enters_chase_on_proximity() {
        let mut tree = sentinel_patrol_chase();
        let mut ctx = StateTreeContext::new();
        ctx.distance_to_target = 20.0;
        tree.tick(&mut ctx, 0.016);
        assert_eq!(tree.active_name(), "Patrol");

        ctx.distance_to_target = 3.0;
        let r = tree.tick(&mut ctx, 0.016);
        assert!(r.transitioned);
        assert_eq!(tree.active_name(), "Chase");
    }

    #[test]
    fn chase_returns_with_hysteresis() {
        let mut tree = sentinel_patrol_chase_ranges(6.0, 14.0);
        let mut ctx = StateTreeContext::new();
        ctx.distance_to_target = 2.0;
        tree.tick(&mut ctx, 0.016);
        assert_eq!(tree.active_name(), "Chase");

        // Still inside exit radius — stay chasing.
        ctx.distance_to_target = 10.0;
        tree.tick(&mut ctx, 0.016);
        assert_eq!(tree.active_name(), "Chase");

        ctx.distance_to_target = 15.0;
        tree.tick(&mut ctx, 0.016);
        assert_eq!(tree.active_name(), "Patrol");
    }

    #[test]
    fn hierarchical_parent_transition() {
        // Active leaf has no matching transition; parent Root does.
        let mut b = StateTreeBuilder::new();
        let root = b.add_state("Root");
        let a = b.add_state("A");
        let done = b.add_state("Done");
        b.set_parent(a, root);
        b.add_transition(a, done, TransitionCondition::Never);
        b.add_transition(root, done, TransitionCondition::FlagSet("escape"));
        let mut tree = b.build();
        let mut ctx = StateTreeContext::new();
        tree.force_transition(a, &mut ctx);
        ctx.set_flag("escape", true);
        let r = tree.evaluate(&mut ctx);
        assert!(r.transitioned);
        assert_eq!(tree.active_name(), "Done");
    }
}
