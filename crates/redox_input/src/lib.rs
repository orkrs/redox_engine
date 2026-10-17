//! Input subsystem for the RedOx Engine.
//!
//! Provides keyboard, mouse, and action-mapping abstractions built on top of
//! `winit` events. The primary ECS resource is [`InputState`], which is updated
//! once per frame during the `Input` stage.

#![warn(missing_docs)]

pub mod action;
pub mod buffer;
pub mod context;
pub mod gamepad;
pub mod keyboard;
pub mod mouse;
pub mod rebind;
pub mod state;
pub mod text;

pub use action::{
    ActionBinding, ActionKind, ActionMap, Devices, apply_deadzone_1d, apply_deadzone_2d,
};
pub use buffer::{CoyoteTimer, InputBuffer};
pub use context::InputContexts;
#[cfg(feature = "gamepad")]
pub use gamepad::GilrsBackend;
pub use gamepad::{GamepadAxis, GamepadButton, GamepadId, GamepadsState, PadState};
pub use keyboard::KeyboardState;
pub use mouse::MouseState;
pub use rebind::{RebindOutcome, Rebinder};
pub use state::InputState;
pub use text::{Preedit, TextInput};
