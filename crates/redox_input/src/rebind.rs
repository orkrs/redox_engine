//! Runtime rebinding of actions: editing, conflict detection, RON persistence and
//! "press a key to rebind" capture.

use serde::{Deserialize, Serialize};
use winit::keyboard::KeyCode;

use crate::action::{ActionBinding, ActionMap};
use crate::gamepad::GamepadButton;
use crate::mouse::MouseButton;

#[derive(Serialize, Deserialize)]
struct BindingsFile {
    actions: Vec<(String, Vec<ActionBinding>)>,
}

/// Whether two bindings listen to at least one common physical input.
fn shares_input(a: &ActionBinding, b: &ActionBinding) -> bool {
    fn keys(binding: &ActionBinding) -> Vec<KeyCode> {
        match binding {
            ActionBinding::Key(k) => vec![*k],
            ActionBinding::Axis { positive, negative } => vec![*positive, *negative],
            ActionBinding::Mouse(_) | ActionBinding::PadButton(_) | ActionBinding::PadAxis(_) => {
                Vec::new()
            }
        }
    }
    match (a, b) {
        (ActionBinding::Mouse(x), ActionBinding::Mouse(y)) => x == y,
        (ActionBinding::PadButton(x), ActionBinding::PadButton(y)) => x == y,
        (ActionBinding::PadAxis(x), ActionBinding::PadAxis(y)) => x == y,
        _ => {
            let other = keys(b);
            keys(a).iter().any(|k| other.contains(k))
        }
    }
}

impl ActionMap {
    /// Names of all actions that have at least one binding, sorted.
    pub fn actions(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.bindings.keys().map(String::as_str).collect();
        names.sort_unstable();
        names
    }

    /// Replaces every binding of `name` (an empty list removes the action).
    pub fn set_bindings(&mut self, name: &str, bindings: Vec<ActionBinding>) {
        if bindings.is_empty() {
            self.bindings.remove(name);
        } else {
            self.bindings.insert(name.to_string(), bindings);
        }
    }

    /// Replaces the binding at `index` of `name`; returns `false` when it does not exist.
    pub fn rebind(&mut self, name: &str, index: usize, binding: ActionBinding) -> bool {
        match self.bindings.get_mut(name).and_then(|v| v.get_mut(index)) {
            Some(slot) => {
                *slot = binding;
                true
            }
            None => false,
        }
    }

    /// Removes the binding at `index` of `name` and returns it.
    pub fn remove_binding(&mut self, name: &str, index: usize) -> Option<ActionBinding> {
        let list = self.bindings.get_mut(name)?;
        if index >= list.len() {
            return None;
        }
        let removed = list.remove(index);
        if list.is_empty() {
            self.bindings.remove(name);
        }
        Some(removed)
    }

    /// Actions other than `except` that already use any physical input of `binding`.
    ///
    /// Call it before [`Self::rebind`] to warn about (or resolve) double assignments.
    pub fn conflicts(&self, binding: &ActionBinding, except: &str) -> Vec<String> {
        let mut found: Vec<String> = self
            .bindings
            .iter()
            .filter(|(name, _)| name.as_str() != except)
            .filter(|(_, list)| list.iter().any(|b| shares_input(b, binding)))
            .map(|(name, _)| name.clone())
            .collect();
        found.sort_unstable();
        found
    }

    /// Serializes all bindings to RON (sorted by action name, so the output is stable).
    pub fn to_ron(&self) -> Result<String, ron::Error> {
        let file = BindingsFile {
            actions: self
                .actions()
                .into_iter()
                .map(|n| (n.to_string(), self.bindings_for(n).to_vec()))
                .collect(),
        };
        ron::ser::to_string_pretty(&file, ron::ser::PrettyConfig::new())
    }

    /// Parses bindings written by [`Self::to_ron`].
    pub fn from_ron(text: &str) -> Result<Self, ron::error::SpannedError> {
        let file: BindingsFile = ron::from_str(text)?;
        let mut map = Self::new();
        for (name, list) in file.actions {
            map.set_bindings(&name, list);
        }
        Ok(map)
    }

    /// Overlays `user` on `self`: actions present in `user` replace the defaults of the
    /// same name, all other actions keep their defaults.
    pub fn with_overrides(mut self, user: &ActionMap) -> Self {
        for (name, list) in &user.bindings {
            self.bindings.insert(name.clone(), list.clone());
        }
        self
    }
}

/// Captures the next pressed input and stores it as a binding ("press a key to rebind").
///
/// ```ignore
/// let mut rebinder = Rebinder::default();
/// rebinder.start("Jump", 0);                       // UI: the user clicked "rebind Jump"
/// // for every key press event afterwards:
/// if let Some(done) = rebinder.feed_key(&mut map, key) { /* refresh the UI */ }
/// ```
#[derive(Debug, Default)]
pub struct Rebinder {
    target: Option<(String, usize)>,
}

/// Result of a completed capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebindOutcome {
    /// The action that was rebound.
    pub action: String,
    /// Other actions that now share the new input.
    pub conflicts: Vec<String>,
}

impl Rebinder {
    /// Starts waiting for input that will replace binding `index` of `action`.
    pub fn start(&mut self, action: &str, index: usize) {
        self.target = Some((action.to_string(), index));
    }

    /// Stops waiting without changing anything.
    pub fn cancel(&mut self) {
        self.target = None;
    }

    /// Whether a capture is in progress.
    pub fn is_capturing(&self) -> bool {
        self.target.is_some()
    }

    /// Feeds a pressed key. `Escape` cancels. For an `Axis` binding the key replaces the
    /// positive side.
    pub fn feed_key(&mut self, map: &mut ActionMap, key: KeyCode) -> Option<RebindOutcome> {
        if key == KeyCode::Escape {
            self.cancel();
            return None;
        }
        let (action, index) = self.target.take()?;
        let new = match map.bindings_for(&action).get(index)? {
            ActionBinding::Axis { negative, .. } => ActionBinding::Axis {
                positive: key,
                negative: *negative,
            },
            _ => ActionBinding::Key(key),
        };
        let conflicts = map.conflicts(&new, &action);
        map.rebind(&action, index, new)
            .then_some(RebindOutcome { action, conflicts })
    }

    /// Feeds a pressed mouse button. An axis binding cannot take a button, so the capture
    /// keeps waiting in that case.
    pub fn feed_mouse(
        &mut self,
        map: &mut ActionMap,
        button: MouseButton,
    ) -> Option<RebindOutcome> {
        let (action, index) = self.target.take()?;
        if matches!(
            map.bindings_for(&action).get(index)?,
            ActionBinding::Axis { .. }
        ) {
            self.target = Some((action, index));
            return None;
        }
        let new = ActionBinding::Mouse(button);
        let conflicts = map.conflicts(&new, &action);
        map.rebind(&action, index, new)
            .then_some(RebindOutcome { action, conflicts })
    }

    /// Feeds a pressed gamepad button (from
    /// [`GamepadsState::first_pressed`](crate::gamepad::GamepadsState::first_pressed)). Like a
    /// mouse button, it cannot replace a keyboard axis, so an axis capture keeps waiting.
    pub fn feed_pad_button(
        &mut self,
        map: &mut ActionMap,
        button: GamepadButton,
    ) -> Option<RebindOutcome> {
        let (action, index) = self.target.take()?;
        if matches!(
            map.bindings_for(&action).get(index)?,
            ActionBinding::Axis { .. }
        ) {
            self.target = Some((action, index));
            return None;
        }
        let new = ActionBinding::PadButton(button);
        let conflicts = map.conflicts(&new, &action);
        map.rebind(&action, index, new)
            .then_some(RebindOutcome { action, conflicts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ActionMap {
        let mut map = ActionMap::new();
        map.add("Jump", ActionBinding::Key(KeyCode::Space));
        map.add("Fire", ActionBinding::Mouse(MouseButton::Left));
        map.add(
            "Move",
            ActionBinding::Axis {
                positive: KeyCode::KeyD,
                negative: KeyCode::KeyA,
            },
        );
        map
    }

    #[test]
    fn rebinding_and_removal() {
        let mut map = sample();
        assert!(map.rebind("Jump", 0, ActionBinding::Key(KeyCode::KeyJ)));
        assert!(!map.rebind("Jump", 5, ActionBinding::Key(KeyCode::KeyK)));
        assert!(!map.rebind("Nope", 0, ActionBinding::Key(KeyCode::KeyK)));
        assert_eq!(
            map.bindings_for("Jump"),
            [ActionBinding::Key(KeyCode::KeyJ)]
        );
        assert!(map.remove_binding("Jump", 0).is_some());
        assert!(map.bindings_for("Jump").is_empty());
        assert_eq!(map.actions(), ["Fire", "Move"]);
    }

    #[test]
    fn conflicts_detect_shared_inputs() {
        let map = sample();
        assert_eq!(
            map.conflicts(&ActionBinding::Key(KeyCode::KeyA), "Jump"),
            ["Move"]
        );
        assert!(
            map.conflicts(&ActionBinding::Key(KeyCode::KeyA), "Move")
                .is_empty()
        );
        assert_eq!(
            map.conflicts(&ActionBinding::Mouse(MouseButton::Left), "x"),
            ["Fire"]
        );
        assert!(
            map.conflicts(&ActionBinding::Mouse(MouseButton::Right), "x")
                .is_empty()
        );
    }

    #[test]
    fn ron_round_trip_and_overrides() {
        let map = sample();
        let text = map.to_ron().unwrap();
        let back = ActionMap::from_ron(&text).unwrap();
        assert_eq!(back.actions(), map.actions());
        for name in map.actions() {
            assert_eq!(back.bindings_for(name), map.bindings_for(name));
        }
        assert_eq!(text, back.to_ron().unwrap(), "output is stable");

        let mut user = ActionMap::new();
        user.add("Jump", ActionBinding::Key(KeyCode::KeyW));
        let merged = map.with_overrides(&user);
        assert_eq!(
            merged.bindings_for("Jump"),
            [ActionBinding::Key(KeyCode::KeyW)]
        );
        assert_eq!(
            merged.bindings_for("Fire").len(),
            1,
            "untouched defaults stay"
        );
        assert!(ActionMap::from_ron("garbage").is_err());
    }

    #[test]
    fn rebinder_captures_the_next_key() {
        let mut map = sample();
        let mut rebinder = Rebinder::default();
        assert!(rebinder.feed_key(&mut map, KeyCode::KeyX).is_none(), "idle");

        rebinder.start("Jump", 0);
        assert!(rebinder.is_capturing());
        let done = rebinder.feed_key(&mut map, KeyCode::KeyD).unwrap();
        assert_eq!(done.action, "Jump");
        assert_eq!(done.conflicts, ["Move"]);
        assert!(!rebinder.is_capturing());
        assert_eq!(
            map.bindings_for("Jump"),
            [ActionBinding::Key(KeyCode::KeyD)]
        );

        rebinder.start("Jump", 0);
        assert!(rebinder.feed_key(&mut map, KeyCode::Escape).is_none());
        assert!(!rebinder.is_capturing(), "escape cancels");
        assert_eq!(
            map.bindings_for("Jump"),
            [ActionBinding::Key(KeyCode::KeyD)]
        );
    }

    #[test]
    fn rebinder_axis_takes_the_positive_key_and_rejects_mouse() {
        let mut map = sample();
        let mut rebinder = Rebinder::default();
        rebinder.start("Move", 0);
        assert!(rebinder.feed_mouse(&mut map, MouseButton::Right).is_none());
        assert!(
            rebinder.is_capturing(),
            "an axis cannot take a mouse button"
        );
        rebinder.feed_key(&mut map, KeyCode::KeyE).unwrap();
        assert_eq!(
            map.bindings_for("Move"),
            [ActionBinding::Axis {
                positive: KeyCode::KeyE,
                negative: KeyCode::KeyA
            }]
        );

        rebinder.start("Fire", 0);
        rebinder.feed_mouse(&mut map, MouseButton::Middle).unwrap();
        assert_eq!(
            map.bindings_for("Fire"),
            [ActionBinding::Mouse(MouseButton::Middle)]
        );
    }
}
