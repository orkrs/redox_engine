//! RedOx Editor Core — non-visual business logic for RedOx Studio.

#![warn(missing_docs)]

/// Selection set and primary selection.
pub mod selection;
/// Undo and redo commands.
pub mod undo;

pub use selection::{Selection, Visibility};
pub use undo::{UndoCommand, UndoHotkey, UndoStack};
