//! RedOx Editor Core — non-visual business logic for RedOx Studio.
//!
//! The crate has no egui, window or GPU dependencies. [`EditorContext`] bundles the shared
//! editor state (world, registry, selection, undo, log, status, commands) so that the editor
//! can be driven and tested without a window.

#![warn(missing_docs)]

/// Built-in commands that work on the domain state (create, duplicate, delete).
pub mod builtin_commands;
/// Editor commands addressed by identifier and the registry that runs them.
pub mod command;
/// Shared handles to the editor state.
pub mod context;
/// File and project commands; they queue requests for the editor host.
pub mod file_commands;
/// Pure rules of the file dialogs: prompts, Save target, menu availability, window title.
pub mod file_state;
/// Marker for entities that exist only inside the editor.
pub mod editor_only;
/// Console log ring buffer.
pub mod log;
/// Project manifest `project.ron`, project creation, opening and the recent-projects list.
pub mod project;
/// Starter templates for new projects.
pub mod project_template;
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
