//! Gamepad state, plus an optional `gilrs` backend (feature `gamepad`).
//!
//! [`GamepadsState`] is plain data fed by any source: the [`GilrsBackend`], a test, or a
//! replay. Sticks get a radial-free per-axis deadzone with rescaling; triggers are 0..1.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::action::apply_deadzone_1d;

/// Default stick deadzone.
pub const DEFAULT_DEADZONE: f32 = 0.15;

/// Identifies a connected gamepad for the lifetime of the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GamepadId(pub u32);

/// Face, shoulder, stick and d-pad buttons (layout follows an Xbox pad).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GamepadButton {
    /// Bottom face button (A / Cross).
    South,
    /// Right face button (B / Circle).
    East,
    /// Left face button (X / Square).
    West,
    /// Top face button (Y / Triangle).
    North,
    /// Left bumper.
    LeftShoulder,
    /// Right bumper.
    RightShoulder,
    /// Left trigger pressed as a button.
    LeftTrigger,
    /// Right trigger pressed as a button.
    RightTrigger,
    /// Back / Select.
    Select,
    /// Start.
    Start,
    /// Left stick click.
    LeftThumb,
    /// Right stick click.
    RightThumb,
    /// D-pad up.
    DPadUp,
    /// D-pad down.
    DPadDown,
    /// D-pad left.
    DPadLeft,
    /// D-pad right.
    DPadRight,
}

/// Analog axes. Stick axes are -1..1 (up and right positive); triggers are 0..1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GamepadAxis {
    /// Left stick, horizontal.
    LeftStickX,
    /// Left stick, vertical.
    LeftStickY,
    /// Right stick, horizontal.
    RightStickX,
    /// Right stick, vertical.
    RightStickY,
    /// Left trigger.
    LeftTrigger,
    /// Right trigger.
    RightTrigger,
}

impl GamepadAxis {
    fn is_trigger(self) -> bool {
        matches!(self, Self::LeftTrigger | Self::RightTrigger)
    }
}

/// State of one pad.
#[derive(Debug, Clone, Default)]
pub struct PadState {
    held: HashSet<GamepadButton>,
    just_pressed: HashSet<GamepadButton>,
    just_released: HashSet<GamepadButton>,
    axes: BTreeMap<u8, f32>,
}

fn axis_key(axis: GamepadAxis) -> u8 {
    axis as u8
}

impl PadState {
    /// Whether `button` is held.
    pub fn is_pressed(&self, button: GamepadButton) -> bool {
        self.held.contains(&button)
    }

    /// Whether `button` went down this frame.
    pub fn just_pressed(&self, button: GamepadButton) -> bool {
        self.just_pressed.contains(&button)
    }

    /// Whether `button` went up this frame.
    pub fn just_released(&self, button: GamepadButton) -> bool {
        self.just_released.contains(&button)
    }

    /// Axis value after the deadzone (0.0 when never reported).
    pub fn axis(&self, axis: GamepadAxis) -> f32 {
        self.axes.get(&axis_key(axis)).copied().unwrap_or(0.0)
    }
}

/// All connected gamepads (ECS resource part of [`crate::InputState`]).
#[derive(Debug, Clone)]
pub struct GamepadsState {
    pads: BTreeMap<GamepadId, PadState>,
    deadzone: f32,
}

impl Default for GamepadsState {
    fn default() -> Self {
        Self::new()
    }
}

impl GamepadsState {
    /// Creates an empty state with [`DEFAULT_DEADZONE`].
    pub fn new() -> Self {
        Self {
            pads: BTreeMap::new(),
            deadzone: DEFAULT_DEADZONE,
        }
    }

    /// Sets the stick deadzone (clamped to 0..1).
    pub fn set_deadzone(&mut self, deadzone: f32) {
        self.deadzone = deadzone.clamp(0.0, 0.99);
    }

    /// Clears the per-frame edges. Call once per frame like the other devices.
    pub fn begin_frame(&mut self) {
        for pad in self.pads.values_mut() {
            pad.just_pressed.clear();
            pad.just_released.clear();
        }
    }

    /// Registers a pad (idempotent).
    pub fn connect(&mut self, id: GamepadId) {
        self.pads.entry(id).or_default();
    }

    /// Forgets a pad and everything it held.
    pub fn disconnect(&mut self, id: GamepadId) {
        self.pads.remove(&id);
    }

    /// Ids of the connected pads, ascending.
    pub fn connected(&self) -> impl Iterator<Item = GamepadId> + '_ {
        self.pads.keys().copied()
    }

    /// State of one pad.
    pub fn pad(&self, id: GamepadId) -> Option<&PadState> {
        self.pads.get(&id)
    }

    /// Records a button going down (connects the pad when needed).
    pub fn press(&mut self, id: GamepadId, button: GamepadButton) {
        let pad = self.pads.entry(id).or_default();
        if pad.held.insert(button) {
            pad.just_pressed.insert(button);
        }
    }

    /// Records a button going up.
    pub fn release(&mut self, id: GamepadId, button: GamepadButton) {
        let pad = self.pads.entry(id).or_default();
        if pad.held.remove(&button) {
            pad.just_released.insert(button);
        }
    }

    /// Records a raw axis value; sticks go through the deadzone, triggers are clamped to 0..1.
    pub fn set_axis(&mut self, id: GamepadId, axis: GamepadAxis, raw: f32) {
        let value = if axis.is_trigger() {
            raw.clamp(0.0, 1.0)
        } else {
            apply_deadzone_1d(raw.clamp(-1.0, 1.0), self.deadzone)
        };
        self.pads
            .entry(id)
            .or_default()
            .axes
            .insert(axis_key(axis), value);
    }

    /// Whether any pad holds `button`.
    pub fn any_pressed(&self, button: GamepadButton) -> bool {
        self.pads.values().any(|p| p.is_pressed(button))
    }

    /// Whether any pad pressed `button` this frame.
    pub fn any_just_pressed(&self, button: GamepadButton) -> bool {
        self.pads.values().any(|p| p.just_pressed(button))
    }

    /// The axis value of largest magnitude across all pads.
    pub fn any_axis(&self, axis: GamepadAxis) -> f32 {
        self.pads
            .values()
            .map(|p| p.axis(axis))
            .fold(0.0, |best, v| if v.abs() > best.abs() { v } else { best })
    }

    /// A button currently held on any pad (used to capture a binding).
    pub fn first_pressed(&self) -> Option<GamepadButton> {
        self.pads
            .values()
            .flat_map(|p| p.just_pressed.iter().copied())
            .min_by_key(|b| *b as u8)
    }
}

#[cfg(feature = "gamepad")]
pub use backend::GilrsBackend;

#[cfg(feature = "gamepad")]
mod backend {
    use super::{GamepadAxis, GamepadButton, GamepadId, GamepadsState};

    /// Maps a gilrs button.
    pub fn map_button(button: gilrs::Button) -> Option<GamepadButton> {
        use gilrs::Button as B;
        Some(match button {
            B::South => GamepadButton::South,
            B::East => GamepadButton::East,
            B::West => GamepadButton::West,
            B::North => GamepadButton::North,
            B::LeftTrigger => GamepadButton::LeftShoulder,
            B::RightTrigger => GamepadButton::RightShoulder,
            B::LeftTrigger2 => GamepadButton::LeftTrigger,
            B::RightTrigger2 => GamepadButton::RightTrigger,
            B::Select => GamepadButton::Select,
            B::Start => GamepadButton::Start,
            B::LeftThumb => GamepadButton::LeftThumb,
            B::RightThumb => GamepadButton::RightThumb,
            B::DPadUp => GamepadButton::DPadUp,
            B::DPadDown => GamepadButton::DPadDown,
            B::DPadLeft => GamepadButton::DPadLeft,
            B::DPadRight => GamepadButton::DPadRight,
            _ => return None,
        })
    }

    /// Maps a gilrs axis.
    pub fn map_axis(axis: gilrs::Axis) -> Option<GamepadAxis> {
        use gilrs::Axis as A;
        Some(match axis {
            A::LeftStickX => GamepadAxis::LeftStickX,
            A::LeftStickY => GamepadAxis::LeftStickY,
            A::RightStickX => GamepadAxis::RightStickX,
            A::RightStickY => GamepadAxis::RightStickY,
            A::LeftZ => GamepadAxis::LeftTrigger,
            A::RightZ => GamepadAxis::RightTrigger,
            _ => return None,
        })
    }

    /// Polls `gilrs` and feeds a [`GamepadsState`].
    pub struct GilrsBackend {
        gilrs: gilrs::Gilrs,
    }

    impl GilrsBackend {
        /// Starts the backend; `None` (and a log line) when the platform offers no gamepad API.
        pub fn new() -> Option<Self> {
            match gilrs::Gilrs::new() {
                Ok(gilrs) => Some(Self { gilrs }),
                Err(e) => {
                    log::warn!("gamepad support unavailable: {e}");
                    None
                }
            }
        }

        fn id(id: gilrs::GamepadId) -> GamepadId {
            GamepadId(usize::from(id) as u32)
        }

        /// Drains pending events into `state`, and registers pads that were already plugged in.
        pub fn poll(&mut self, state: &mut GamepadsState) {
            for (id, _) in self.gilrs.gamepads() {
                state.connect(Self::id(id));
            }
            while let Some(event) = self.gilrs.next_event() {
                use gilrs::EventType as E;
                let id = Self::id(event.id);
                match event.event {
                    E::Connected => state.connect(id),
                    E::Disconnected => state.disconnect(id),
                    E::ButtonPressed(b, _) => {
                        if let Some(b) = map_button(b) {
                            state.press(id, b);
                        }
                    }
                    E::ButtonReleased(b, _) => {
                        if let Some(b) = map_button(b) {
                            state.release(id, b);
                        }
                    }
                    E::AxisChanged(a, value, _) => {
                        if let Some(a) = map_axis(a) {
                            state.set_axis(id, a, value);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn every_supported_gilrs_button_maps_to_its_counterpart() {
            use gilrs::Button as B;
            let table = [
                (B::South, GamepadButton::South),
                (B::East, GamepadButton::East),
                (B::West, GamepadButton::West),
                (B::North, GamepadButton::North),
                (B::LeftTrigger, GamepadButton::LeftShoulder),
                (B::RightTrigger, GamepadButton::RightShoulder),
                (B::LeftTrigger2, GamepadButton::LeftTrigger),
                (B::RightTrigger2, GamepadButton::RightTrigger),
                (B::Select, GamepadButton::Select),
                (B::Start, GamepadButton::Start),
                (B::LeftThumb, GamepadButton::LeftThumb),
                (B::RightThumb, GamepadButton::RightThumb),
                (B::DPadUp, GamepadButton::DPadUp),
                (B::DPadDown, GamepadButton::DPadDown),
                (B::DPadLeft, GamepadButton::DPadLeft),
                (B::DPadRight, GamepadButton::DPadRight),
            ];
            for (raw, expected) in table {
                assert_eq!(map_button(raw), Some(expected), "{raw:?}");
            }
        }

        #[test]
        fn unmapped_gilrs_buttons_are_ignored() {
            assert_eq!(map_button(gilrs::Button::Mode), None);
            assert_eq!(map_button(gilrs::Button::C), None);
            assert_eq!(map_button(gilrs::Button::Z), None);
            assert_eq!(map_button(gilrs::Button::Unknown), None);
        }

        #[test]
        fn every_supported_gilrs_axis_maps_to_its_counterpart() {
            use gilrs::Axis as A;
            let table = [
                (A::LeftStickX, GamepadAxis::LeftStickX),
                (A::LeftStickY, GamepadAxis::LeftStickY),
                (A::RightStickX, GamepadAxis::RightStickX),
                (A::RightStickY, GamepadAxis::RightStickY),
                (A::LeftZ, GamepadAxis::LeftTrigger),
                (A::RightZ, GamepadAxis::RightTrigger),
            ];
            for (raw, expected) in table {
                assert_eq!(map_axis(raw), Some(expected), "{raw:?}");
            }
            assert_eq!(map_axis(gilrs::Axis::DPadX), None);
            assert_eq!(map_axis(gilrs::Axis::DPadY), None);
        }

        #[test]
        fn backend_start_never_panics() {
            // May be `None` on machines without gamepad support; polling must still work.
            if let Some(mut backend) = GilrsBackend::new() {
                let mut state = GamepadsState::new();
                backend.poll(&mut state);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    const PAD: GamepadId = GamepadId(0);

    #[test]
    fn buttons_report_edges_once_per_frame() {
        let mut pads = GamepadsState::new();
        pads.press(PAD, GamepadButton::South);
        pads.press(PAD, GamepadButton::South); // repeat: no second edge
        assert!(pads.any_pressed(GamepadButton::South));
        assert!(pads.pad(PAD).unwrap().just_pressed(GamepadButton::South));
        assert_eq!(pads.first_pressed(), Some(GamepadButton::South));

        pads.begin_frame();
        assert!(pads.any_pressed(GamepadButton::South));
        assert!(!pads.any_just_pressed(GamepadButton::South));

        pads.release(PAD, GamepadButton::South);
        assert!(pads.pad(PAD).unwrap().just_released(GamepadButton::South));
        assert!(!pads.any_pressed(GamepadButton::South));
    }

    #[test]
    fn sticks_use_a_rescaled_deadzone_and_triggers_clamp() {
        let mut pads = GamepadsState::new();
        pads.set_deadzone(0.2);
        pads.set_axis(PAD, GamepadAxis::LeftStickX, 0.1);
        assert_eq!(pads.any_axis(GamepadAxis::LeftStickX), 0.0);
        pads.set_axis(PAD, GamepadAxis::LeftStickX, 0.6);
        assert!((pads.any_axis(GamepadAxis::LeftStickX) - 0.5).abs() < 1e-5);
        pads.set_axis(PAD, GamepadAxis::LeftStickX, -3.0);
        assert!((pads.any_axis(GamepadAxis::LeftStickX) + 1.0).abs() < 1e-5);
        pads.set_axis(PAD, GamepadAxis::RightTrigger, 1.7);
        assert_eq!(pads.any_axis(GamepadAxis::RightTrigger), 1.0);
        pads.set_axis(PAD, GamepadAxis::LeftTrigger, -0.4);
        assert_eq!(pads.any_axis(GamepadAxis::LeftTrigger), 0.0);
    }

    #[test]
    fn several_pads_combine_and_disconnect_clears() {
        let mut pads = GamepadsState::new();
        let (a, b) = (GamepadId(1), GamepadId(2));
        pads.set_deadzone(0.0);
        pads.set_axis(a, GamepadAxis::LeftStickX, 0.3);
        pads.set_axis(b, GamepadAxis::LeftStickX, -0.8);
        assert!((pads.any_axis(GamepadAxis::LeftStickX) + 0.8).abs() < 1e-5);
        assert_eq!(pads.connected().collect::<Vec<_>>(), [a, b]);
        pads.press(b, GamepadButton::Start);
        pads.disconnect(b);
        assert!(!pads.any_pressed(GamepadButton::Start));
        assert!((pads.any_axis(GamepadAxis::LeftStickX) - 0.3).abs() < 1e-5);
    }

    #[test]
    fn default_deadzone_is_applied_to_new_state() {
        let mut pads = GamepadsState::new();
        pads.set_axis(PAD, GamepadAxis::RightStickY, DEFAULT_DEADZONE);
        assert_eq!(pads.any_axis(GamepadAxis::RightStickY), 0.0);

        pads.set_axis(PAD, GamepadAxis::RightStickY, 0.2);
        let expected = (0.2 - DEFAULT_DEADZONE) / (1.0 - DEFAULT_DEADZONE);
        assert!((pads.any_axis(GamepadAxis::RightStickY) - expected).abs() < 1e-5);
    }

    #[test]
    fn deadzone_is_clamped_to_a_valid_range() {
        let mut pads = GamepadsState::new();
        pads.set_deadzone(-1.0);
        pads.set_axis(PAD, GamepadAxis::LeftStickY, 0.05);
        assert!((pads.any_axis(GamepadAxis::LeftStickY) - 0.05).abs() < 1e-5);

        pads.set_deadzone(5.0);
        pads.set_axis(PAD, GamepadAxis::LeftStickY, 0.995);
        assert!((pads.any_axis(GamepadAxis::LeftStickY) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn negative_stick_values_use_the_same_deadzone() {
        let mut pads = GamepadsState::new();
        pads.set_deadzone(0.2);
        pads.set_axis(PAD, GamepadAxis::RightStickX, -0.1);
        assert_eq!(pads.any_axis(GamepadAxis::RightStickX), 0.0);

        pads.set_axis(PAD, GamepadAxis::RightStickX, -0.6);
        assert!((pads.any_axis(GamepadAxis::RightStickX) + 0.5).abs() < 1e-5);
    }

    #[test]
    fn triggers_ignore_the_stick_deadzone() {
        let mut pads = GamepadsState::new();
        pads.set_deadzone(0.9);
        pads.set_axis(PAD, GamepadAxis::LeftTrigger, 0.5);
        assert_eq!(pads.any_axis(GamepadAxis::LeftTrigger), 0.5);
    }

    #[test]
    fn unreported_axes_and_empty_state_read_as_zero() {
        let pads = GamepadsState::new();
        assert_eq!(pads.any_axis(GamepadAxis::LeftStickX), 0.0);
        assert!(pads.pad(PAD).is_none());

        let mut with_pad = GamepadsState::new();
        with_pad.connect(PAD);
        let state = with_pad.pad(PAD).unwrap();
        assert_eq!(state.axis(GamepadAxis::RightTrigger), 0.0);
        assert!(!state.is_pressed(GamepadButton::East));
    }

    #[test]
    fn pressing_an_unknown_pad_connects_it() {
        let mut pads = GamepadsState::new();
        pads.press(GamepadId(9), GamepadButton::West);
        assert_eq!(pads.connected().collect::<Vec<_>>(), [GamepadId(9)]);
        assert!(pads.any_just_pressed(GamepadButton::West));
    }

    #[test]
    fn connect_is_idempotent_and_keeps_existing_state() {
        let mut pads = GamepadsState::new();
        pads.press(PAD, GamepadButton::North);
        pads.connect(PAD);
        assert_eq!(pads.connected().count(), 1);
        assert!(pads.any_pressed(GamepadButton::North));
    }

    #[test]
    fn disconnecting_an_unknown_pad_is_a_no_op() {
        let mut pads = GamepadsState::new();
        pads.connect(PAD);
        pads.disconnect(GamepadId(42));
        assert_eq!(pads.connected().collect::<Vec<_>>(), [PAD]);
    }

    #[test]
    fn disconnect_only_forgets_the_removed_pad() {
        let mut pads = GamepadsState::new();
        let (a, b) = (GamepadId(1), GamepadId(2));
        pads.press(a, GamepadButton::South);
        pads.press(b, GamepadButton::South);
        pads.set_axis(b, GamepadAxis::RightTrigger, 0.7);

        pads.disconnect(a);
        assert!(pads.pad(a).is_none());
        assert!(
            pads.any_pressed(GamepadButton::South),
            "pad b still holds it"
        );
        assert!((pads.any_axis(GamepadAxis::RightTrigger) - 0.7).abs() < 1e-5);
    }

    #[test]
    fn releasing_a_button_that_is_not_held_produces_no_edge() {
        let mut pads = GamepadsState::new();
        pads.connect(PAD);
        pads.release(PAD, GamepadButton::Start);
        assert!(!pads.pad(PAD).unwrap().just_released(GamepadButton::Start));
    }

    #[test]
    fn just_released_is_cleared_by_begin_frame() {
        let mut pads = GamepadsState::new();
        pads.press(PAD, GamepadButton::Select);
        pads.begin_frame();
        pads.release(PAD, GamepadButton::Select);
        assert!(pads.pad(PAD).unwrap().just_released(GamepadButton::Select));

        pads.begin_frame();
        assert!(!pads.pad(PAD).unwrap().just_released(GamepadButton::Select));
    }

    #[test]
    fn any_helpers_look_across_every_pad() {
        let mut pads = GamepadsState::new();
        let (a, b) = (GamepadId(1), GamepadId(2));
        pads.connect(a);
        pads.connect(b);
        assert!(!pads.any_pressed(GamepadButton::East));

        pads.press(b, GamepadButton::East);
        assert!(pads.any_pressed(GamepadButton::East));
        assert!(pads.any_just_pressed(GamepadButton::East));

        pads.begin_frame();
        assert!(pads.any_pressed(GamepadButton::East));
        assert!(!pads.any_just_pressed(GamepadButton::East));
    }

    #[test]
    fn any_axis_is_zero_without_pads() {
        let pads = GamepadsState::new();
        assert_eq!(pads.any_axis(GamepadAxis::LeftTrigger), 0.0);
    }

    #[test]
    fn first_pressed_reports_the_lowest_button_from_this_frame_only() {
        let mut pads = GamepadsState::new();
        let (a, b) = (GamepadId(1), GamepadId(2));
        pads.press(a, GamepadButton::East);
        pads.press(b, GamepadButton::South);
        assert_eq!(pads.first_pressed(), Some(GamepadButton::South));

        pads.begin_frame();
        assert_eq!(pads.first_pressed(), None);
    }

    #[test]
    fn default_state_has_no_pads() {
        let pads = GamepadsState::default();
        assert_eq!(pads.connected().count(), 0);
        assert_eq!(pads.first_pressed(), None);
    }

    #[test]
    fn pad_state_reports_held_and_edge_flags() {
        let mut pads = GamepadsState::new();
        pads.press(PAD, GamepadButton::DPadUp);
        let state = pads.pad(PAD).unwrap();
        assert!(state.is_pressed(GamepadButton::DPadUp));
        assert!(state.just_pressed(GamepadButton::DPadUp));
        assert!(!state.just_released(GamepadButton::DPadUp));
    }
}
