//! Mouse state tracking.

use std::collections::HashSet;

/// Identifies a mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum MouseButton {
    /// Primary (left) button.
    Left,
    /// Secondary (right) button.
    Right,
    /// Middle button (wheel click).
    Middle,
    /// Back button (thumb button, usually "browser back").
    Back,
    /// Forward button (thumb button, usually "browser forward").
    Forward,
    /// Any other button, identified by its platform index.
    Other(u16),
}

impl From<winit::event::MouseButton> for MouseButton {
    fn from(btn: winit::event::MouseButton) -> Self {
        match btn {
            winit::event::MouseButton::Left => Self::Left,
            winit::event::MouseButton::Right => Self::Right,
            winit::event::MouseButton::Middle => Self::Middle,
            winit::event::MouseButton::Back => Self::Back,
            winit::event::MouseButton::Forward => Self::Forward,
            winit::event::MouseButton::Other(id) => Self::Other(id),
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
    /// Creates a state with the cursor at the origin and no buttons pressed.
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

    #[test]
    fn just_released_is_true_only_on_the_frame_after_release() {
        let mut mouse = MouseState::new();
        mouse.press(MouseButton::Right);
        mouse.begin_frame();
        assert!(!mouse.just_released(MouseButton::Right), "still held");

        mouse.release(MouseButton::Right);
        assert!(mouse.just_released(MouseButton::Right));
        assert!(!mouse.is_pressed(MouseButton::Right));

        mouse.begin_frame();
        assert!(!mouse.just_released(MouseButton::Right), "edge cleared");
        assert!(!mouse.is_pressed(MouseButton::Right));
    }

    #[test]
    fn repeated_press_in_one_frame_produces_a_single_edge() {
        let mut mouse = MouseState::new();
        mouse.begin_frame();
        mouse.press(MouseButton::Middle);
        mouse.press(MouseButton::Middle);
        assert!(mouse.just_pressed(MouseButton::Middle));

        mouse.begin_frame();
        assert!(mouse.is_pressed(MouseButton::Middle));
        assert!(!mouse.just_pressed(MouseButton::Middle));
    }

    #[test]
    fn releasing_an_unpressed_button_is_harmless() {
        let mut mouse = MouseState::new();
        mouse.release(MouseButton::Left);
        assert!(!mouse.is_pressed(MouseButton::Left));
        assert!(!mouse.just_released(MouseButton::Left));
    }

    #[test]
    fn buttons_are_tracked_independently_including_other_indices() {
        let mut mouse = MouseState::new();
        mouse.press(MouseButton::Left);
        mouse.press(MouseButton::Other(4));
        mouse.release(MouseButton::Left);

        assert!(!mouse.is_pressed(MouseButton::Left));
        assert!(mouse.is_pressed(MouseButton::Other(4)));
        assert!(!mouse.is_pressed(MouseButton::Other(5)));
    }

    #[test]
    fn position_delta_accumulates_within_a_frame() {
        let mut mouse = MouseState::new();
        mouse.set_position(10.0, 10.0);
        mouse.set_position(15.0, 5.0);
        mouse.set_position(12.0, 8.0);

        assert_eq!(mouse.position, [12.0, 8.0]);
        assert_eq!(mouse.delta, [12.0, 8.0]);
    }

    #[test]
    fn delta_resets_each_frame_but_position_persists() {
        let mut mouse = MouseState::new();
        mouse.set_position(3.0, 4.0);
        mouse.begin_frame();

        assert_eq!(mouse.delta, [0.0, 0.0]);
        assert_eq!(mouse.position, [3.0, 4.0]);
    }

    #[test]
    fn scroll_accumulates_within_a_frame_and_resets() {
        let mut mouse = MouseState::new();
        mouse.scroll(0.0, 1.0);
        mouse.scroll(0.5, -2.0);
        assert_eq!(mouse.scroll_delta, [0.5, -1.0]);

        mouse.begin_frame();
        assert_eq!(mouse.scroll_delta, [0.0, 0.0]);
    }

    #[test]
    fn default_is_the_same_as_new() {
        let mouse = MouseState::default();
        assert_eq!(mouse.position, [0.0, 0.0]);
        assert_eq!(mouse.delta, [0.0, 0.0]);
        assert!(!mouse.is_pressed(MouseButton::Left));
    }

    #[test]
    fn winit_buttons_convert_to_mouse_buttons() {
        use winit::event::MouseButton as Winit;

        assert_eq!(MouseButton::from(Winit::Left), MouseButton::Left);
        assert_eq!(MouseButton::from(Winit::Right), MouseButton::Right);
        assert_eq!(MouseButton::from(Winit::Middle), MouseButton::Middle);
        assert_eq!(MouseButton::from(Winit::Other(7)), MouseButton::Other(7));
    }

    #[test]
    fn back_and_forward_buttons_are_distinct() {
        let back = MouseButton::from(winit::event::MouseButton::Back);
        let forward = MouseButton::from(winit::event::MouseButton::Forward);
        assert_ne!(back, forward);
    }
}
