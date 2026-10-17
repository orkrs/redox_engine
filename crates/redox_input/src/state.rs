//! Combined input state — the primary ECS resource for input queries.

use crate::action::{ActionKind, ActionMap, Devices};
use crate::context::InputContexts;
use crate::gamepad::GamepadsState;
use crate::keyboard::KeyboardState;
use crate::mouse::MouseState;
use crate::text::TextInput;

/// ECS resource that aggregates all input state for the current frame.
///
/// Insert this into the `World` and update it each frame from winit events.
#[derive(Debug, Clone)]
pub struct InputState {
    /// Keyboard state.
    pub keyboard: KeyboardState,
    /// Mouse state.
    pub mouse: MouseState,
    /// Gamepads (fed by [`crate::gamepad::GilrsBackend`] or by hand).
    pub gamepads: GamepadsState,
    /// Typed text and IME composition.
    pub text: TextInput,
    /// Base action bindings, consulted when no active context claims an action.
    pub actions: ActionMap,
    /// Layered contexts that take precedence over [`Self::actions`].
    pub contexts: InputContexts,
}

impl InputState {
    /// Creates a new empty input state.
    pub fn new() -> Self {
        Self {
            keyboard: KeyboardState::new(),
            mouse: MouseState::new(),
            gamepads: GamepadsState::new(),
            text: TextInput::new(),
            actions: ActionMap::new(),
            contexts: InputContexts::new(),
        }
    }

    /// Call at the start of each frame before processing new events.
    pub fn begin_frame(&mut self) {
        self.keyboard.begin_frame();
        self.mouse.begin_frame();
        self.gamepads.begin_frame();
        self.text.begin_frame();
    }

    /// Evaluates a named action against the current input state.
    ///
    /// Active [contexts](InputContexts) are consulted first, then the base map.
    pub fn action(&self, name: &str) -> ActionKind {
        let devices = Devices {
            keyboard: &self.keyboard,
            mouse: &self.mouse,
            gamepads: &self.gamepads,
        };
        self.contexts
            .evaluate(name, &devices)
            .unwrap_or_else(|| self.actions.evaluate_with(name, &devices))
    }

    /// Convenience: returns `true` if a digital action is active.
    pub fn action_active(&self, name: &str) -> bool {
        match self.action(name) {
            ActionKind::Digital(v) => v,
            ActionKind::Analog(v) => v.abs() > 0.5,
        }
    }

    /// Convenience: returns the analog value of an action (0.0 for digital false).
    pub fn action_value(&self, name: &str) -> f32 {
        match self.action(name) {
            ActionKind::Digital(v) => {
                if v {
                    1.0
                } else {
                    0.0
                }
            }
            ActionKind::Analog(v) => v,
        }
    }

    /// Process a winit `WindowEvent` and update internal state accordingly.
    pub fn process_window_event(&mut self, event: &winit::event::WindowEvent) {
        use winit::event::{ElementState, WindowEvent};

        self.text.process_window_event(event);

        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                if let winit::keyboard::PhysicalKey::Code(key_code) = event.physical_key {
                    match event.state {
                        ElementState::Pressed => self.keyboard.press(key_code),
                        ElementState::Released => self.keyboard.release(key_code),
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse
                    .set_position(position.x as f32, position.y as f32);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let btn = crate::mouse::MouseButton::from(*button);
                match state {
                    ElementState::Pressed => self.mouse.press(btn),
                    ElementState::Released => self.mouse.release(btn),
                }
            }
            WindowEvent::MouseWheel { delta, .. } => match delta {
                winit::event::MouseScrollDelta::LineDelta(dx, dy) => {
                    self.mouse.scroll(*dx, *dy);
                }
                winit::event::MouseScrollDelta::PixelDelta(pos) => {
                    self.mouse.scroll(pos.x as f32, pos.y as f32);
                }
            },
            _ => {}
        }
    }
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::action::ActionBinding;
    use crate::gamepad::{GamepadAxis, GamepadButton, GamepadId};
    use crate::mouse::MouseButton;
    use winit::event::{Ime, WindowEvent};
    use winit::keyboard::KeyCode;

    fn axis_map() -> ActionMap {
        let mut map = ActionMap::new();
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
    fn new_state_has_no_input_and_default_matches_new() {
        let input = InputState::default();
        assert!(!input.keyboard.is_pressed(KeyCode::Space));
        assert!(!input.mouse.is_pressed(MouseButton::Left));
        assert_eq!(input.gamepads.connected().count(), 0);
        assert!(!input.text.wants_text());
        assert!(!input.action_active("Anything"));
    }

    #[test]
    fn begin_frame_shifts_every_device_into_the_next_frame() {
        let mut input = InputState::new();
        input.keyboard.press(KeyCode::Space);
        input.mouse.press(MouseButton::Left);
        input.mouse.set_position(4.0, 4.0);
        input.gamepads.press(GamepadId(0), GamepadButton::South);
        input.text.set_wants_text(true);
        input.text.type_text("a");

        input.begin_frame();

        assert!(input.keyboard.is_pressed(KeyCode::Space));
        assert!(!input.keyboard.just_pressed(KeyCode::Space));
        assert!(input.mouse.is_pressed(MouseButton::Left));
        assert!(!input.mouse.just_pressed(MouseButton::Left));
        assert_eq!(input.mouse.delta, [0.0, 0.0]);
        assert!(input.gamepads.any_pressed(GamepadButton::South));
        assert!(!input.gamepads.any_just_pressed(GamepadButton::South));
        assert_eq!(input.text.typed(), "");
    }

    #[test]
    fn digital_action_follows_its_key_and_reports_one_or_zero() {
        let mut input = InputState::new();
        input
            .actions
            .add("Jump", ActionBinding::Key(KeyCode::Space));
        assert!(!input.action_active("Jump"));
        assert_eq!(input.action_value("Jump"), 0.0);

        input.keyboard.press(KeyCode::Space);
        assert!(input.action_active("Jump"));
        assert_eq!(input.action_value("Jump"), 1.0);
    }

    #[test]
    fn mouse_and_pad_button_actions_read_their_devices() {
        let mut input = InputState::new();
        input
            .actions
            .add("Fire", ActionBinding::Mouse(MouseButton::Left));
        input
            .actions
            .add("Confirm", ActionBinding::PadButton(GamepadButton::South));

        input.mouse.press(MouseButton::Left);
        input.gamepads.press(GamepadId(3), GamepadButton::South);

        assert!(input.action_active("Fire"));
        assert_eq!(input.action_value("Fire"), 1.0);
        assert!(input.action_active("Confirm"));
    }

    #[test]
    fn axis_action_is_signed_and_cancels_when_both_keys_are_held() {
        let mut input = InputState::new();
        input.actions = axis_map();

        input.keyboard.press(KeyCode::KeyD);
        assert_eq!(input.action_value("Move"), 1.0);
        assert!(input.action_active("Move"));

        input.keyboard.release(KeyCode::KeyD);
        input.keyboard.press(KeyCode::KeyA);
        assert_eq!(input.action_value("Move"), -1.0);
        assert!(input.action_active("Move"));

        input.keyboard.press(KeyCode::KeyD);
        assert_eq!(input.action_value("Move"), 0.0);
        assert!(!input.action_active("Move"));
    }

    #[test]
    fn analog_pad_axis_counts_as_active_only_past_half_travel() {
        let mut input = InputState::new();
        input
            .actions
            .add("Steer", ActionBinding::PadAxis(GamepadAxis::LeftStickX));
        input.gamepads.set_deadzone(0.0);

        input
            .gamepads
            .set_axis(GamepadId(0), GamepadAxis::LeftStickX, 0.3);
        assert!(!input.action_active("Steer"));
        assert!((input.action_value("Steer") - 0.3).abs() < 1e-5);

        input
            .gamepads
            .set_axis(GamepadId(0), GamepadAxis::LeftStickX, -0.8);
        assert!(input.action_active("Steer"));
        assert!((input.action_value("Steer") + 0.8).abs() < 1e-5);
    }

    #[test]
    fn unknown_action_is_inactive_and_zero() {
        let input = InputState::new();
        assert!(matches!(input.action("Nope"), ActionKind::Digital(false)));
        assert_eq!(input.action_value("Nope"), 0.0);
    }

    #[test]
    fn blocking_context_hides_the_base_map_until_popped() {
        let mut input = InputState::new();
        input
            .actions
            .add("Jump", ActionBinding::Key(KeyCode::Space));

        let mut menu = ActionMap::new();
        menu.add("Jump", ActionBinding::Mouse(MouseButton::Left));
        input.contexts.define("menu", menu, true);
        input.contexts.push("menu");

        input.keyboard.press(KeyCode::Space);
        assert!(!input.action_active("Jump"), "the menu swallows the key");

        input.mouse.press(MouseButton::Left);
        assert!(input.action_active("Jump"), "the menu's own binding works");

        input.contexts.pop();
        assert!(input.action_active("Jump"), "base map is back after pop");
    }

    #[test]
    fn non_blocking_context_falls_through_for_actions_it_does_not_claim() {
        let mut input = InputState::new();
        input
            .actions
            .add("Jump", ActionBinding::Key(KeyCode::Space));

        let mut pause = ActionMap::new();
        pause.add("Pause", ActionBinding::Key(KeyCode::Escape));
        input.contexts.define("pause", pause, false);
        input.contexts.push("pause");

        input.keyboard.press(KeyCode::Space);
        assert!(input.action_active("Jump"));
    }

    #[test]
    fn focus_change_events_do_not_touch_device_state() {
        let mut input = InputState::new();
        input.keyboard.press(KeyCode::KeyW);
        input.mouse.set_position(7.0, 9.0);

        input.process_window_event(&WindowEvent::Focused(true));
        input.process_window_event(&WindowEvent::CloseRequested);

        assert!(input.keyboard.is_pressed(KeyCode::KeyW));
        assert_eq!(input.mouse.position, [7.0, 9.0]);
    }

    #[test]
    fn ime_commit_is_typed_only_while_a_text_field_has_focus() {
        let mut input = InputState::new();
        input.process_window_event(&WindowEvent::Ime(Ime::Commit("x".into())));
        assert_eq!(input.text.typed(), "", "no focused field, nothing typed");

        input.text.set_wants_text(true);
        input.process_window_event(&WindowEvent::Ime(Ime::Commit("日本".into())));
        assert_eq!(input.text.typed(), "日本");
    }

    #[test]
    fn ime_preedit_and_disabled_drive_the_composition() {
        let mut input = InputState::new();
        input.text.set_wants_text(true);

        input.process_window_event(&WindowEvent::Ime(Ime::Preedit("に".into(), Some((0, 3)))));
        assert!(input.text.preedit().is_some());

        input.process_window_event(&WindowEvent::Ime(Ime::Disabled));
        assert!(input.text.preedit().is_none());
    }
}
