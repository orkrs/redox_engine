//! Input subsystem for the RedOx Engine.
//!
//! Provides keyboard, mouse, and action-mapping abstractions built on top of
//! `winit` events. The primary ECS resource is [`InputState`], which is updated
//! once per frame during the `Input` stage.

pub mod keyboard;
pub mod mouse;
pub mod action;
pub mod state;
pub mod buffer;

pub use keyboard::KeyboardState;
pub use mouse::MouseState;
pub use action::{ActionMap, ActionKind, ActionBinding, apply_deadzone_1d, apply_deadzone_2d};
pub use state::InputState;
pub use buffer::{CoyoteTimer, InputBuffer};

