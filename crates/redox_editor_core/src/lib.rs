//! RedOx Editor Core — non-visual business logic for RedOx Studio.
//!
//! The crate has no egui, window or GPU dependencies. [`EditorContext`] bundles the shared
//! editor state (world, registry, selection, undo, log, status, commands) so that the editor
//! can be driven and tested without a window.

#![warn(missing_docs)]

/// Autosave of a dirty level into `<project>/.redox/autosave`, on a background writer.
pub mod autosave;
/// Built-in commands that work on the domain state (create, duplicate, delete).
pub mod builtin_commands;
/// Editor commands addressed by identifier and the registry that runs them.
pub mod command;
/// Shared handles to the editor state.
pub mod context;
/// Marker for entities that exist only inside the editor.
pub mod editor_only;
/// Console log ring buffer.
pub mod log;
/// Session lock and crash recovery of an autosaved level.
pub mod recovery;
/// Selection set and primary selection.
pub mod selection;
/// Undo and redo commands.
pub mod undo;

pub use command::{CommandRegistry, EditorCommand};
pub use context::EditorContext;
pub use editor_only::{EditorOnly, editor_only_entities};
pub use log::EditorLog;
pub use selection::{Selection, Visibility};
pub use undo::{UndoCommand, UndoHotkey, UndoStack};
