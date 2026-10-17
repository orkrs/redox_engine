//! Gamepad input for Courier: the same actions as the keyboard.
//!
//! Left stick moves, right stick turns the camera, A (South) or X (West) waves, A also confirms in
//! menus, D-pad moves the menu focus, B (East) or Start goes back / pauses.

use redox_input::{GamepadAxis, GamepadButton, GamepadsState};
use redox_ui::game::Direction;

/// What the pad asks for this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PadInput {
    /// Left stick, right is +1.
    pub move_x: f32,
    /// Left stick, forward is +1.
    pub move_z: f32,
    /// Right stick, right is +1.
    pub camera_turn: f32,
    /// A wave button went down this frame.
    pub wave: bool,
    /// The confirm button went down this frame.
    pub confirm: bool,
    /// The back / pause button went down this frame.
    pub back: bool,
    /// A D-pad direction went down this frame.
    pub nav: Option<Direction>,
}

/// Reads all connected pads (the first one that does something wins per control).
pub fn read(pads: &GamepadsState) -> PadInput {
    let nav = [
        (GamepadButton::DPadUp, Direction::Up),
        (GamepadButton::DPadDown, Direction::Down),
        (GamepadButton::DPadLeft, Direction::Left),
        (GamepadButton::DPadRight, Direction::Right),
    ]
    .into_iter()
    .find(|(button, _)| pads.any_just_pressed(*button))
    .map(|(_, direction)| direction);

    PadInput {
        move_x: pads.any_axis(GamepadAxis::LeftStickX),
        move_z: pads.any_axis(GamepadAxis::LeftStickY),
        camera_turn: pads.any_axis(GamepadAxis::RightStickX),
        wave: pads.any_just_pressed(GamepadButton::South)
            || pads.any_just_pressed(GamepadButton::West),
        confirm: pads.any_just_pressed(GamepadButton::South),
        back: pads.any_just_pressed(GamepadButton::East)
            || pads.any_just_pressed(GamepadButton::Start),
        nav,
    }
}

#[cfg(test)]
mod tests {
    use redox_input::GamepadId;

    use super::*;

    const PAD: GamepadId = GamepadId(1);

    fn pads() -> GamepadsState {
        let mut pads = GamepadsState::new();
        pads.connect(PAD);
        pads
    }

    #[test]
    fn nothing_connected_means_no_input() {
        assert_eq!(read(&GamepadsState::new()), PadInput::default());
    }

    #[test]
    fn sticks_drive_movement_and_camera() {
        let mut pads = pads();
        pads.set_axis(PAD, GamepadAxis::LeftStickX, 0.8);
        pads.set_axis(PAD, GamepadAxis::LeftStickY, -0.5);
        pads.set_axis(PAD, GamepadAxis::RightStickX, 1.0);
        let input = read(&pads);
        assert!((input.move_x - 0.8).abs() < 0.2, "{input:?}");
        assert!(input.move_z < -0.2, "{input:?}");
        assert!(input.camera_turn > 0.9, "{input:?}");
    }

    #[test]
    fn a_small_stick_wobble_is_inside_the_deadzone() {
        let mut pads = pads();
        pads.set_axis(PAD, GamepadAxis::LeftStickX, 0.02);
        assert_eq!(read(&pads).move_x, 0.0);
    }

    #[test]
    fn buttons_are_edges_not_levels() {
        let mut pads = pads();
        pads.press(PAD, GamepadButton::South);
        let first = read(&pads);
        assert!(first.wave && first.confirm);
        pads.begin_frame();
        let held = read(&pads);
        assert!(!held.wave && !held.confirm, "held, not pressed again");
    }

    #[test]
    fn west_waves_but_does_not_confirm() {
        let mut pads = pads();
        pads.press(PAD, GamepadButton::West);
        let input = read(&pads);
        assert!(input.wave && !input.confirm);
    }

    #[test]
    fn east_and_start_go_back() {
        for button in [GamepadButton::East, GamepadButton::Start] {
            let mut pads = pads();
            pads.press(PAD, button);
            assert!(read(&pads).back, "{button:?}");
        }
    }

    #[test]
    fn the_dpad_moves_the_menu_focus() {
        for (button, direction) in [
            (GamepadButton::DPadUp, Direction::Up),
            (GamepadButton::DPadDown, Direction::Down),
            (GamepadButton::DPadLeft, Direction::Left),
            (GamepadButton::DPadRight, Direction::Right),
        ] {
            let mut pads = pads();
            pads.press(PAD, button);
            assert_eq!(read(&pads).nav, Some(direction));
        }
    }
}
