//! Action mapping — abstract game actions bound to physical inputs.

use std::collections::HashMap;
use winit::keyboard::KeyCode;
use crate::mouse::MouseButton;

/// The kind of action value.
#[derive(Debug, Clone)]
pub enum ActionKind {
    /// Boolean — pressed or not (e.g., "Jump").
    Digital(bool),
    /// Scalar — range typically -1.0 to 1.0 (e.g., "MoveHorizontal").
    Analog(f32),
}

/// A physical input binding for an action.
#[derive(Debug, Clone)]
pub enum ActionBinding {
    /// A single key maps to a digital action.
    Key(KeyCode),
    /// Positive / Negative keys map to an analog action in range −1 → +1.
    Axis {
        positive: KeyCode,
        negative: KeyCode,
    },
    /// A mouse button maps to a digital action.
    Mouse(MouseButton),
}

/// Maps named actions to physical bindings.
///
/// # Example
/// ```ignore
/// let mut map = ActionMap::new();
/// map.add("Jump", ActionBinding::Key(KeyCode::Space));
/// map.add("MoveHorizontal", ActionBinding::Axis {
///     positive: KeyCode::KeyD,
///     negative: KeyCode::KeyA,
/// });
/// ```
#[derive(Debug, Clone)]
pub struct ActionMap {
    bindings: HashMap<String, Vec<ActionBinding>>,
}

impl ActionMap {
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
        }
    }

    /// Binds an action name to a physical input.
    ///
    /// Multiple bindings can exist for the same action (e.g., arrow keys AND WASD).
    pub fn add(&mut self, name: &str, binding: ActionBinding) {
        self.bindings
            .entry(name.to_string())
            .or_default()
            .push(binding);
    }

    /// Returns all bindings for the given action.
    pub fn bindings_for(&self, name: &str) -> &[ActionBinding] {
        self.bindings.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Evaluates an action given the current keyboard and mouse state.
    pub fn evaluate(
        &self,
        name: &str,
        keyboard: &crate::keyboard::KeyboardState,
        mouse: &crate::mouse::MouseState,
    ) -> ActionKind {
        let bindings = self.bindings_for(name);
        if bindings.is_empty() {
            return ActionKind::Digital(false);
        }

        for binding in bindings {
            match binding {
                ActionBinding::Key(key) => {
                    if keyboard.is_pressed(*key) {
                        return ActionKind::Digital(true);
                    }
                }
                ActionBinding::Axis { positive, negative } => {
                    let mut value = 0.0_f32;
                    if keyboard.is_pressed(*positive) {
                        value += 1.0;
                    }
                    if keyboard.is_pressed(*negative) {
                        value -= 1.0;
                    }
                    if value != 0.0 {
                        return ActionKind::Analog(value);
                    }
                }
                ActionBinding::Mouse(btn) => {
                    if mouse.is_pressed(*btn) {
                        return ActionKind::Digital(true);
                    }
                }
            }
        }

        // No binding triggered — return default
        match &bindings[0] {
            ActionBinding::Axis { .. } => ActionKind::Analog(0.0),
            _ => ActionKind::Digital(false),
        }
    }
}

impl Default for ActionMap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyboard::KeyboardState;
    use crate::mouse::MouseState;

    #[test]
    fn digital_action() {
        let mut map = ActionMap::new();
        map.add("Jump", ActionBinding::Key(KeyCode::Space));

        let mouse = MouseState::new();
        let mut kb = KeyboardState::new();

        // Not pressed
        match map.evaluate("Jump", &kb, &mouse) {
            ActionKind::Digital(v) => assert!(!v),
            _ => panic!("expected digital"),
        }

        // Press Space
        kb.press(KeyCode::Space);
        match map.evaluate("Jump", &kb, &mouse) {
            ActionKind::Digital(v) => assert!(v),
            _ => panic!("expected digital"),
        }
    }

    #[test]
    fn analog_axis() {
        let mut map = ActionMap::new();
        map.add(
            "Horizontal",
            ActionBinding::Axis {
                positive: KeyCode::KeyD,
                negative: KeyCode::KeyA,
            },
        );

        let mouse = MouseState::new();
        let mut kb = KeyboardState::new();

        // Neither pressed → 0.0
        match map.evaluate("Horizontal", &kb, &mouse) {
            ActionKind::Analog(v) => assert!((v - 0.0).abs() < 0.001),
            _ => panic!("expected analog"),
        }

        // D pressed → 1.0
        kb.press(KeyCode::KeyD);
        match map.evaluate("Horizontal", &kb, &mouse) {
            ActionKind::Analog(v) => assert!((v - 1.0).abs() < 0.001),
            _ => panic!("expected analog"),
        }
    }
}
