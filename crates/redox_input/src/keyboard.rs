//! Keyboard state tracking.

use std::collections::HashSet;
use winit::keyboard::KeyCode;

/// Tracks the current and previous frame key states.
#[derive(Debug, Clone)]
pub struct KeyboardState {
    /// Keys held down this frame.
    current: HashSet<KeyCode>,
    /// Keys held down last frame.
    previous: HashSet<KeyCode>,
}

impl KeyboardState {
    /// Creates an empty keyboard state.
    pub fn new() -> Self {
        Self {
            current: HashSet::new(),
            previous: HashSet::new(),
        }
    }

    /// Call at the start of a new frame to shift current → previous.
    pub fn begin_frame(&mut self) {
        self.previous = self.current.clone();
    }

    /// Records a key press.
    pub fn press(&mut self, key: KeyCode) {
        self.current.insert(key);
    }

    /// Records a key release.
    pub fn release(&mut self, key: KeyCode) {
        self.current.remove(&key);
    }

    /// Returns `true` if the key is currently held down.
    pub fn is_pressed(&self, key: KeyCode) -> bool {
        self.current.contains(&key)
    }

    /// Returns `true` only on the frame the key was first pressed.
    pub fn just_pressed(&self, key: KeyCode) -> bool {
        self.current.contains(&key) && !self.previous.contains(&key)
    }

    /// Returns `true` only on the frame the key was released.
    pub fn just_released(&self, key: KeyCode) -> bool {
        !self.current.contains(&key) && self.previous.contains(&key)
    }

    /// Returns an iterator over all currently pressed keys.
    pub fn pressed_keys(&self) -> impl Iterator<Item = &KeyCode> {
        self.current.iter()
    }
}

impl Default for KeyboardState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn press_and_release() {
        let mut kb = KeyboardState::new();
        kb.press(KeyCode::KeyW);
        assert!(kb.is_pressed(KeyCode::KeyW));
        assert!(!kb.is_pressed(KeyCode::KeyA));

        kb.release(KeyCode::KeyW);
        assert!(!kb.is_pressed(KeyCode::KeyW));
    }

    #[test]
    fn just_pressed_transitions() {
        let mut kb = KeyboardState::new();

        // Frame 1: press W
        kb.begin_frame();
        kb.press(KeyCode::KeyW);
        assert!(kb.just_pressed(KeyCode::KeyW));

        // Frame 2: W still held – not just pressed anymore
        kb.begin_frame();
        kb.press(KeyCode::KeyW);
        assert!(!kb.just_pressed(KeyCode::KeyW));
        assert!(kb.is_pressed(KeyCode::KeyW));

        // Frame 3: release W
        kb.begin_frame();
        // do NOT press W
        kb.release(KeyCode::KeyW);
        assert!(kb.just_released(KeyCode::KeyW));
        assert!(!kb.is_pressed(KeyCode::KeyW));
    }
}
