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
/// Catalog of component types for the add menu and soft dependency prompts.
pub mod component_catalog;
/// Add/remove/copy component values as undo transactions.
pub mod component_commands;
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
/// Folder, lock and related scene markers used by the outliner.
pub mod markers;
/// Lookup of entities by their stable GUID.
pub mod guid;
/// Inspector state without egui: rows, sections, filtering and the edit commands they produce.
pub mod inspector_model;
/// Console log ring buffer.
pub mod log;
/// Project manifest `project.ron`, project creation, opening and the recent-projects list.
pub mod project;
/// Starter templates for new projects.
pub mod project_template;
/// Session lock and crash recovery of an autosaved level.
pub mod recovery;
/// Selection set and primary selection.
pub mod selection;
/// Editor preferences in `editor_settings.ron`.
pub mod settings;
/// Viewport runtime toggles (sky, GI, fog) serialized in settings.
pub mod viewport_prefs;
/// Undo and redo commands.
pub mod undo;

pub use command::{CommandRegistry, EditorCommand};
pub use context::EditorContext;
pub use editor_only::{EditorOnly, editor_only_entities};
pub use markers::{EditorFolder, Locked};
pub use guid::GuidResolver;
pub use log::EditorLog;
pub use selection::{Selection, Visibility};
pub use component_catalog::{
    AddableType, CatalogEntry, ComponentCatalog, DependencyPromptAnswer, DependencyPromptOutcome,
    DependencyWarning, addable_types, dependency_warning_for_add, resolve_dependency_prompt,
};
pub use component_commands::{
    ComponentAddResult, bulk_add_component, component_add, component_add_and_record,
    component_copy_values, component_remove, record_component_transaction,
};
pub use undo::{Clock, SystemClock, UndoCommand, UndoEntry, UndoError, UndoHotkey, UndoStack};
