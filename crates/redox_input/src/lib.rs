//! Input subsystem for the RedOx Engine.
//!
//! Provides keyboard, mouse, and action-mapping abstractions built on top of
//! `winit` events. The primary ECS resource is [`InputState`], which is updated
//! once per frame during the `Input` stage.

pub mod keyboard;
pub mod mouse;
pub mod action;
pub mod state;

pub use keyboard::KeyboardState;
pub use mouse::MouseState;
pub use action::{ActionMap, ActionKind, ActionBinding};
pub use state::InputState;
