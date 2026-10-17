//! Game UI toolkit: a retained widget tree with real layout, localisation, gamepad/keyboard
//! focus navigation and a screen stack. It is separate from the egui debug UI.
//!
//! The parts do not depend on a renderer, so they are tested without a window:
//!
//! * [`mod@layout`]: [`Node`] trees (columns, rows, stacks, labels, buttons, bars) laid out in a
//!   virtual 1920x1080 space that scales to any window and respects a safe-area margin.
//! * [`loc`]: [`Localization`] tables with a fallback language and `{name}` arguments.
//! * [`focus`]: [`Focus`] moves a selection between buttons with the d-pad or arrow keys.
//! * [`screens`]: [`ScreenStack`] for menus, pause and dialogs; only the top screen gets input.
//! * [`paint`]: draws a laid-out tree with egui's painter (fonts come from egui).
//!
//! ```
//! use redox_ui::game::*;
//! let tree = Node::column(8.0).padding(16.0).child(Node::label("Hello")).child(Node::button("ok", "OK"));
//! let ui = layout(&tree, Rect::new(0.0, 0.0, 400.0, 300.0), &approx_text);
//! assert!(ui.rect_of("ok").is_some());
//! ```

pub mod focus;
pub mod layout;
pub mod loc;
pub mod paint;
pub mod screens;

pub use focus::{Direction, Focus};
pub use layout::{
    Align, Color, Dim, Kind, Laid, LaidItem, LaidUi, Node, Rect, Style, Text, UiScale, approx_text,
    layout, layout_localized,
};
pub use loc::Localization;
pub use screens::{Screen, ScreenStack};
