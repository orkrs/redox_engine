//! Combined input state — the primary ECS resource for input queries.

use crate::action::{ActionKind, ActionMap};
use crate::keyboard::KeyboardState;
use crate::mouse::MouseState;

/// ECS resource that aggregates all input state for the current frame.
///
/// Insert this into the `World` and update it each frame from winit events.
#[derive(Debug, Clone)]
pub struct InputState {
    /// Keyboard state.
    pub keyboard: KeyboardState,
    /// Mouse state.
    pub mouse: MouseState,
    /// Action bindings.
    pub actions: ActionMap,
}

impl InputState {
    /// Creates a new empty input state.
    pub fn new() -> Self {
        Self {
            keyboard: KeyboardState::new(),
            mouse: MouseState::new(),
            actions: ActionMap::new(),
        }
    }

    /// Call at the start of each frame before processing new events.
    pub fn begin_frame(&mut self) {
        self.keyboard.begin_frame();
        self.mouse.begin_frame();
    }

    /// Evaluates a named action against the current input state.
    pub fn action(&self, name: &str) -> ActionKind {
        self.actions.evaluate(name, &self.keyboard, &self.mouse)
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
            ActionKind::Digital(v) => if v { 1.0 } else { 0.0 },
            ActionKind::Analog(v) => v,
        }
    }

    /// Process a winit `WindowEvent` and update internal state accordingly.
    pub fn process_window_event(&mut self, event: &winit::event::WindowEvent) {
        use winit::event::{ElementState, WindowEvent};

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
                self.mouse.set_position(position.x as f32, position.y as f32);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let btn = crate::mouse::MouseButton::from(*button);
                match state {
                    ElementState::Pressed => self.mouse.press(btn),
                    ElementState::Released => self.mouse.release(btn),
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                match delta {
                    winit::event::MouseScrollDelta::LineDelta(dx, dy) => {
                        self.mouse.scroll(*dx, *dy);
                    }
                    winit::event::MouseScrollDelta::PixelDelta(pos) => {
                        self.mouse.scroll(pos.x as f32, pos.y as f32);
                    }
                }
            }
            _ => {}
        }
    }
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}
