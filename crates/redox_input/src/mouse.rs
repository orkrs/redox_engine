//! Mouse state tracking.

use std::collections::HashSet;

/// Identifies a mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other(u16),
}

impl From<winit::event::MouseButton> for MouseButton {
    fn from(btn: winit::event::MouseButton) -> Self {
        match btn {
            winit::event::MouseButton::Left => Self::Left,
            winit::event::MouseButton::Right => Self::Right,
            winit::event::MouseButton::Middle => Self::Middle,
            winit::event::MouseButton::Other(id) => Self::Other(id),
            _ => Self::Other(0),
        }
    }
}

/// Tracks the mouse position, delta, buttons, and scroll wheel.
#[derive(Debug, Clone)]
pub struct MouseState {
    /// Current cursor position in logical pixels.
    pub position: [f32; 2],
    /// Movement delta since last frame.
    pub delta: [f32; 2],
    /// Scroll delta (horizontal, vertical).
    pub scroll_delta: [f32; 2],
    /// Buttons held this frame.
    current_buttons: HashSet<MouseButton>,
    /// Buttons held last frame.
    previous_buttons: HashSet<MouseButton>,
}

impl MouseState {
    pub fn new() -> Self {
        Self {
            position: [0.0; 2],
            delta: [0.0; 2],
            scroll_delta: [0.0; 2],
            current_buttons: HashSet::new(),
            previous_buttons: HashSet::new(),
        }
    }

    /// Call at the start of a new frame.
    pub fn begin_frame(&mut self) {
        self.previous_buttons = self.current_buttons.clone();
        self.delta = [0.0; 2];
        self.scroll_delta = [0.0; 2];
    }

    /// Records cursor movement.
    pub fn set_position(&mut self, x: f32, y: f32) {
        self.delta[0] += x - self.position[0];
        self.delta[1] += y - self.position[1];
        self.position = [x, y];
    }

    /// Records a mouse button press.
    pub fn press(&mut self, button: MouseButton) {
        self.current_buttons.insert(button);
    }

    /// Records a mouse button release.
    pub fn release(&mut self, button: MouseButton) {
        self.current_buttons.remove(&button);
    }

    /// Records scroll input.
    pub fn scroll(&mut self, dx: f32, dy: f32) {
        self.scroll_delta[0] += dx;
        self.scroll_delta[1] += dy;
    }

    /// Returns `true` if the given button is currently held.
    pub fn is_pressed(&self, button: MouseButton) -> bool {
        self.current_buttons.contains(&button)
    }

    /// Returns `true` only on the frame the button was first pressed.
    pub fn just_pressed(&self, button: MouseButton) -> bool {
        self.current_buttons.contains(&button) && !self.previous_buttons.contains(&button)
    }

    /// Returns `true` only on the frame the button was released.
    pub fn just_released(&self, button: MouseButton) -> bool {
        !self.current_buttons.contains(&button) && self.previous_buttons.contains(&button)
    }
}

impl Default for MouseState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_button_transitions() {
        let mut mouse = MouseState::new();

        mouse.begin_frame();
        mouse.press(MouseButton::Left);
        assert!(mouse.just_pressed(MouseButton::Left));

        mouse.begin_frame();
        // still held
        assert!(!mouse.just_pressed(MouseButton::Left));
        assert!(mouse.is_pressed(MouseButton::Left));

        mouse.begin_frame();
        mouse.release(MouseButton::Left);
        assert!(mouse.just_released(MouseButton::Left));
    }

    #[test]
    fn mouse_position_delta() {
        let mut mouse = MouseState::new();
        mouse.set_position(100.0, 200.0);
        mouse.begin_frame(); // resets delta
        mouse.set_position(110.0, 190.0);
        assert!((mouse.delta[0] - 10.0).abs() < 0.001);
        assert!((mouse.delta[1] - (-10.0)).abs() < 0.001);
    }
}
