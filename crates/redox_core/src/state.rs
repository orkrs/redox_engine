//! Application states with enter/exit hooks (menu, loading, playing, ...).
//!
//! A state type is any small `Copy + Eq + Hash` enum. [`init_state`] inserts the
//! [`State`] / [`NextState`] resources; systems request a change with
//! [`NextState::set`] and [`apply_state_transition`] (run once per frame by
//! [`AppBuilder::add_state`](crate::AppBuilder::add_state)) performs it: exit hooks of the
//! old state, the switch, then enter hooks of the new state. Hooks may request another
//! transition, which is applied in the same call (bounded by [`MAX_CHAINED_TRANSITIONS`]).
//!
//! Gate systems on a state with [`in_state`] and
//! [`Dispatcher::add_system_if`](crate::Dispatcher::add_system_if).

use std::fmt::Debug;
use std::hash::Hash;

use redox_ecs::World;
use std::collections::HashMap;

/// Upper bound on transitions applied by one [`apply_state_transition`] call.
pub const MAX_CHAINED_TRANSITIONS: usize = 8;

/// Marker for types usable as application state.
pub trait AppState: Copy + Eq + Hash + Debug + Send + Sync + 'static {}

impl<T: Copy + Eq + Hash + Debug + Send + Sync + 'static> AppState for T {}

/// The current state (resource).
#[derive(Debug)]
pub struct State<S: AppState> {
    current: S,
    previous: Option<S>,
}

impl<S: AppState> State<S> {
    /// The active state.
    pub fn get(&self) -> S {
        self.current
    }

    /// The state that was active before the last transition, if any.
    pub fn previous(&self) -> Option<S> {
        self.previous
    }
}

/// A requested transition (resource). The last request before the next
/// [`apply_state_transition`] wins.
#[derive(Debug)]
pub struct NextState<S: AppState>(Option<S>);

impl<S: AppState> NextState<S> {
    /// Requests a switch to `state`.
    pub fn set(&mut self, state: S) {
        self.0 = Some(state);
    }

    /// The pending request, if any.
    pub fn pending(&self) -> Option<S> {
        self.0
    }
}

type Hook = Box<dyn FnMut(&mut World) + Send + Sync>;

/// Enter/exit hooks for one state type (resource).
pub struct StateHooks<S: AppState> {
    enter: HashMap<S, Vec<Hook>>,
    exit: HashMap<S, Vec<Hook>>,
    /// The initial state's enter hooks have not run yet.
    initial_pending: bool,
}

/// Inserts [`State`], [`NextState`] and the hook table. The initial state's enter hooks
/// run on the first [`apply_state_transition`].
pub fn init_state<S: AppState>(world: &mut World, initial: S) {
    world.insert_resource(State {
        current: initial,
        previous: None,
    });
    world.insert_resource(NextState::<S>(None));
    world.insert_resource(StateHooks::<S> {
        enter: HashMap::new(),
        exit: HashMap::new(),
        initial_pending: true,
    });
}

fn hooks_mut<S: AppState>(world: &mut World) -> &mut StateHooks<S> {
    world
        .get_resource_mut::<StateHooks<S>>()
        .expect("call init_state / AppBuilder::add_state before registering state hooks")
}

/// Runs `hook` whenever `state` is entered.
pub fn on_enter<S: AppState>(
    world: &mut World,
    state: S,
    hook: impl FnMut(&mut World) + Send + Sync + 'static,
) {
    hooks_mut::<S>(world)
        .enter
        .entry(state)
        .or_default()
        .push(Box::new(hook));
}

/// Runs `hook` whenever `state` is left.
pub fn on_exit<S: AppState>(
    world: &mut World,
    state: S,
    hook: impl FnMut(&mut World) + Send + Sync + 'static,
) {
    hooks_mut::<S>(world)
        .exit
        .entry(state)
        .or_default()
        .push(Box::new(hook));
}

fn run_hooks<S: AppState>(world: &mut World, table: &mut HashMap<S, Vec<Hook>>, state: S) {
    if let Some(hooks) = table.get_mut(&state) {
        for hook in hooks {
            hook(world);
        }
    }
}

/// Applies pending transitions. Returns how many state changes happened.
///
/// Requesting the state that is already active is ignored (no exit/enter).
pub fn apply_state_transition<S: AppState>(world: &mut World) -> usize {
    let Some(mut hooks) = world.remove_resource::<StateHooks<S>>() else {
        return 0;
    };
    let mut changes = 0;

    if hooks.initial_pending {
        hooks.initial_pending = false;
        if let Some(initial) = world.get_resource::<State<S>>().map(State::get) {
            run_hooks(world, &mut hooks.enter, initial);
        }
    }

    for _ in 0..MAX_CHAINED_TRANSITIONS {
        let next = world
            .get_resource_mut::<NextState<S>>()
            .and_then(|n| n.0.take());
        let Some(next) = next else { break };
        let Some(current) = world.get_resource::<State<S>>().map(State::get) else {
            break;
        };
        if next == current {
            continue;
        }
        run_hooks(world, &mut hooks.exit, current);
        if let Some(state) = world.get_resource_mut::<State<S>>() {
            state.previous = Some(current);
            state.current = next;
        }
        run_hooks(world, &mut hooks.enter, next);
        changes += 1;
    }

    if world
        .get_resource::<NextState<S>>()
        .is_some_and(|n| n.0.is_some())
    {
        log::warn!(
            "state transition chain for {} exceeded {MAX_CHAINED_TRANSITIONS}; the rest waits for the next frame",
            std::any::type_name::<S>()
        );
    }
    world.insert_resource(hooks);
    changes
}

/// Condition for `add_system_if`: true while `state` is active.
pub fn in_state<S: AppState, Ctx>(state: S) -> impl Fn(&World, &Ctx) -> bool + 'static {
    move |world, _| {
        world
            .get_resource::<State<S>>()
            .is_some_and(|s| s.get() == state)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
    enum Mode {
        Menu,
        Loading,
        Playing,
    }

    type Log = Arc<Mutex<Vec<&'static str>>>;

    fn logged(world: &mut World, log: &Log, state: Mode, enter: &'static str, exit: &'static str) {
        let l = log.clone();
        on_enter(world, state, move |_| l.lock().unwrap().push(enter));
        let l = log.clone();
        on_exit(world, state, move |_| l.lock().unwrap().push(exit));
    }

    fn setup() -> (World, Log) {
        let mut world = World::new();
        init_state(&mut world, Mode::Menu);
        let log = Log::default();
        logged(&mut world, &log, Mode::Menu, "enter menu", "exit menu");
        logged(
            &mut world,
            &log,
            Mode::Loading,
            "enter loading",
            "exit loading",
        );
        logged(
            &mut world,
            &log,
            Mode::Playing,
            "enter playing",
            "exit playing",
        );
        (world, log)
    }

    fn request(world: &mut World, mode: Mode) {
        world
            .get_resource_mut::<NextState<Mode>>()
            .unwrap()
            .set(mode);
    }

    #[test]
    fn initial_state_enters_on_first_apply_only() {
        let (mut world, log) = setup();
        assert_eq!(apply_state_transition::<Mode>(&mut world), 0);
        assert_eq!(apply_state_transition::<Mode>(&mut world), 0);
        assert_eq!(*log.lock().unwrap(), ["enter menu"]);
    }

    #[test]
    fn transition_runs_exit_then_enter_and_tracks_previous() {
        let (mut world, log) = setup();
        apply_state_transition::<Mode>(&mut world);
        request(&mut world, Mode::Loading);
        assert_eq!(apply_state_transition::<Mode>(&mut world), 1);
        let state = world.get_resource::<State<Mode>>().unwrap();
        assert_eq!(
            (state.get(), state.previous()),
            (Mode::Loading, Some(Mode::Menu))
        );
        assert_eq!(
            *log.lock().unwrap(),
            ["enter menu", "exit menu", "enter loading"]
        );
    }

    #[test]
    fn requesting_the_active_state_is_a_no_op() {
        let (mut world, log) = setup();
        apply_state_transition::<Mode>(&mut world);
        request(&mut world, Mode::Menu);
        assert_eq!(apply_state_transition::<Mode>(&mut world), 0);
        assert_eq!(log.lock().unwrap().len(), 1);
    }

    #[test]
    fn hooks_can_chain_transitions_in_one_call() {
        let (mut world, log) = setup();
        on_enter(&mut world, Mode::Loading, |w| {
            w.get_resource_mut::<NextState<Mode>>()
                .unwrap()
                .set(Mode::Playing);
        });
        apply_state_transition::<Mode>(&mut world);
        request(&mut world, Mode::Loading);
        assert_eq!(apply_state_transition::<Mode>(&mut world), 2);
        assert_eq!(
            world.get_resource::<State<Mode>>().unwrap().get(),
            Mode::Playing
        );
        assert_eq!(log.lock().unwrap().last(), Some(&"enter playing"));
    }

    #[test]
    fn endless_ping_pong_is_bounded() {
        let (mut world, _) = setup();
        on_enter(&mut world, Mode::Loading, |w| {
            w.get_resource_mut::<NextState<Mode>>()
                .unwrap()
                .set(Mode::Playing);
        });
        on_enter(&mut world, Mode::Playing, |w| {
            w.get_resource_mut::<NextState<Mode>>()
                .unwrap()
                .set(Mode::Loading);
        });
        apply_state_transition::<Mode>(&mut world);
        request(&mut world, Mode::Loading);
        assert_eq!(
            apply_state_transition::<Mode>(&mut world),
            MAX_CHAINED_TRANSITIONS
        );
    }

    #[test]
    fn in_state_condition_follows_the_state() {
        let (mut world, _) = setup();
        let cond = in_state::<Mode, ()>(Mode::Playing);
        assert!(!cond(&world, &()));
        request(&mut world, Mode::Playing);
        apply_state_transition::<Mode>(&mut world);
        assert!(cond(&world, &()));
    }
}
