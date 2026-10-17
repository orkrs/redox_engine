//! Input contexts: layered action maps (gameplay, menu, dialogue, ...).
//!
//! A context is a named [`ActionMap`]. Pushed contexts form a stack; an action is resolved by
//! the topmost context that binds it. A context created with `blocks_lower = true` hides every
//! context below it (a pause menu that must not move the character), otherwise unbound actions
//! fall through to the layers underneath and finally to [`InputState::actions`](crate::InputState).

use std::collections::HashMap;

use crate::action::{ActionKind, ActionMap, Devices};

#[derive(Debug, Clone)]
struct Context {
    map: ActionMap,
    blocks_lower: bool,
}

/// Registered contexts and the active stack.
#[derive(Debug, Clone, Default)]
pub struct InputContexts {
    contexts: HashMap<String, Context>,
    stack: Vec<String>,
}

impl InputContexts {
    /// Creates an empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers (or replaces) a context. It is inactive until [`Self::push`]ed.
    pub fn define(&mut self, name: &str, map: ActionMap, blocks_lower: bool) {
        self.contexts
            .insert(name.to_string(), Context { map, blocks_lower });
    }

    /// Mutable access to a context's bindings (for rebinding UIs).
    pub fn map_mut(&mut self, name: &str) -> Option<&mut ActionMap> {
        self.contexts.get_mut(name).map(|c| &mut c.map)
    }

    /// Activates a context on top of the stack. Returns `false` for unknown names; pushing an
    /// already active context moves it to the top.
    pub fn push(&mut self, name: &str) -> bool {
        if !self.contexts.contains_key(name) {
            return false;
        }
        self.stack.retain(|n| n != name);
        self.stack.push(name.to_string());
        true
    }

    /// Deactivates the topmost context and returns its name.
    pub fn pop(&mut self) -> Option<String> {
        self.stack.pop()
    }

    /// Deactivates a context wherever it is in the stack.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.stack.len();
        self.stack.retain(|n| n != name);
        self.stack.len() != before
    }

    /// Whether `name` is on the stack.
    pub fn is_active(&self, name: &str) -> bool {
        self.stack.iter().any(|n| n == name)
    }

    /// Active contexts, bottom first.
    pub fn active(&self) -> &[String] {
        &self.stack
    }

    /// Resolves `action` through the stack. `None` means no active context claimed it and the
    /// caller should consult the base map; `Some(Digital(false))` means a blocking context
    /// swallowed it.
    pub fn evaluate(&self, action: &str, devices: &Devices<'_>) -> Option<ActionKind> {
        for name in self.stack.iter().rev() {
            let Some(context) = self.contexts.get(name) else {
                continue;
            };
            if !context.map.bindings_for(action).is_empty() {
                return Some(context.map.evaluate_with(action, devices));
            }
            if context.blocks_lower {
                return Some(ActionKind::Digital(false));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use winit::keyboard::KeyCode;

    use super::*;
    use crate::action::ActionBinding;
    use crate::gamepad::GamepadsState;
    use crate::keyboard::KeyboardState;
    use crate::mouse::MouseState;

    fn map(entries: &[(&str, KeyCode)]) -> ActionMap {
        let mut m = ActionMap::new();
        for (name, key) in entries {
            m.add(name, ActionBinding::Key(*key));
        }
        m
    }

    fn eval(c: &InputContexts, kb: &KeyboardState, action: &str) -> Option<bool> {
        let mouse = MouseState::new();
        let pads = GamepadsState::new();
        let devices = Devices {
            keyboard: kb,
            mouse: &mouse,
            gamepads: &pads,
        };
        c.evaluate(action, &devices).map(|k| match k {
            ActionKind::Digital(v) => v,
            ActionKind::Analog(v) => v != 0.0,
        })
    }

    #[test]
    fn top_context_wins_and_pop_restores_the_lower_one() {
        let mut c = InputContexts::new();
        c.define("gameplay", map(&[("Confirm", KeyCode::KeyE)]), false);
        c.define("menu", map(&[("Confirm", KeyCode::Enter)]), false);
        let mut kb = KeyboardState::new();
        kb.press(KeyCode::KeyE);

        assert_eq!(eval(&c, &kb, "Confirm"), None, "nothing active");
        assert!(c.push("gameplay"));
        assert_eq!(eval(&c, &kb, "Confirm"), Some(true));
        assert!(c.push("menu"));
        assert_eq!(
            eval(&c, &kb, "Confirm"),
            Some(false),
            "menu rebinds Confirm to Enter"
        );
        assert_eq!(c.pop().as_deref(), Some("menu"));
        assert_eq!(eval(&c, &kb, "Confirm"), Some(true));
    }

    #[test]
    fn non_blocking_contexts_fall_through_and_blocking_ones_swallow() {
        let mut c = InputContexts::new();
        c.define("gameplay", map(&[("Jump", KeyCode::Space)]), false);
        c.define("hud", map(&[("Map", KeyCode::KeyM)]), false);
        c.define("pause", map(&[("Resume", KeyCode::Escape)]), true);
        let mut kb = KeyboardState::new();
        kb.press(KeyCode::Space);
        c.push("gameplay");
        c.push("hud");
        assert_eq!(eval(&c, &kb, "Jump"), Some(true), "falls through the HUD");
        assert_eq!(eval(&c, &kb, "Unbound"), None);

        c.push("pause");
        assert_eq!(
            eval(&c, &kb, "Jump"),
            Some(false),
            "blocked by the pause menu"
        );
        assert_eq!(eval(&c, &kb, "Unbound"), Some(false));
    }

    #[test]
    fn stack_management() {
        let mut c = InputContexts::new();
        c.define("a", ActionMap::new(), false);
        c.define("b", ActionMap::new(), false);
        assert!(!c.push("missing"));
        c.push("a");
        c.push("b");
        c.push("a"); // moves to the top instead of duplicating
        assert_eq!(c.active(), ["b", "a"]);
        assert!(c.remove("b"));
        assert!(!c.remove("b"));
        assert!(c.is_active("a") && !c.is_active("b"));
        assert!(c.map_mut("a").is_some() && c.map_mut("zzz").is_none());
    }
}
