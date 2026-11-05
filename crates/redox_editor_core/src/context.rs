//! The headless editor state shared by every front end.

use crate::builtin_commands::{CreateEntity, DeleteEntity, DuplicateEntity};
use crate::command::CommandRegistry;
use crate::log::EditorLog;
use crate::selection::Selection;
use crate::undo::UndoStack;
use parking_lot::Mutex;
use redox_ecs::World;
use redox_reflect::TypeRegistry;
use std::sync::Arc;

/// Shared handles to the editor state. Cloning copies the handles, not the data.
///
/// The context knows nothing about windows, GPU, egui, the viewport or play-in-editor.
#[derive(Clone)]
pub struct EditorContext {
    /// The ECS world being edited.
    pub world: Arc<Mutex<World>>,
    /// Registered component types, used for inspector edits and snapshots.
    pub registry: Arc<Mutex<TypeRegistry>>,
    /// Selected entities.
    pub selection: Arc<Mutex<Selection>>,
    /// Undo and redo history.
    pub undo: Arc<Mutex<UndoStack>>,
    /// Console lines.
    pub log: EditorLog,
    /// Text of the status bar; a front end reads it and acts on it.
    pub status: Arc<Mutex<String>>,
    /// Commands that run by identifier.
    pub commands: CommandRegistry,
}

impl EditorContext {
    /// Creates a context with an empty world and selection, an empty log and the built-in
    /// commands (`create_entity`, `duplicate`, `delete`). The undo history keeps the default
    /// 100 steps, as the editor always did.
    pub fn new(registry: TypeRegistry) -> Self {
        let mut commands = CommandRegistry::new();
        commands.register(CreateEntity);
        commands.register(DuplicateEntity);
        commands.register(DeleteEntity);
        Self {
            world: Arc::new(Mutex::new(World::new())),
            registry: Arc::new(Mutex::new(registry)),
            selection: Arc::new(Mutex::new(Selection::default())),
            undo: Arc::new(Mutex::new(UndoStack::default())),
            log: EditorLog::new(),
            status: Arc::new(Mutex::new(String::new())),
            commands,
        }
    }

    /// Whether the edited document differs from its last saved state, as tracked by the undo
    /// history.
    pub fn is_dirty(&self) -> bool {
        self.undo.lock().is_dirty()
    }
}
