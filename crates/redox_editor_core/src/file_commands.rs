//! File and project commands (`file_new`, `file_save`, `project_open`, ...).
//!
//! The commands do not touch files: each one queues a [`FileRequest`] that the editor host takes
//! out of the [`FileRequestQueue`] and performs. Levels live in `redox_editor_scene`, which this
//! crate cannot depend on, so the work happens in the host.

use crate::command::{CommandRegistry, EditorCommand};
use crate::context::EditorContext;
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::Arc;

/// A file or project operation requested by a command, a menu item or a hotkey.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileRequest {
    /// Start a new empty level.
    NewLevel,
    /// Open a level from a path.
    OpenLevel,
    /// Save the current level.
    Save,
    /// Save the current level to a new path.
    SaveAs,
    /// Close the current level.
    CloseLevel,
    /// Create a new project.
    NewProject,
    /// Open a project from a path.
    OpenProject,
}

/// Requests waiting for the host. Clones share the same queue.
#[derive(Clone, Default)]
pub struct FileRequestQueue {
    pending: Arc<Mutex<VecDeque<FileRequest>>>,
}

impl FileRequestQueue {
    /// Creates an empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `request` at the back of the queue.
    pub fn push(&self, request: FileRequest) {
        self.pending.lock().push_back(request);
    }

    /// Removes and returns all waiting requests, oldest first.
    pub fn take_all(&self) -> Vec<FileRequest> {
        self.pending.lock().drain(..).collect()
    }
}

/// Identifier, palette label and request of every file command.
const FILE_COMMANDS: [(&str, &str, FileRequest); 7] = [
    ("file_new", "> New Level", FileRequest::NewLevel),
    ("file_open", "> Open Level", FileRequest::OpenLevel),
    ("file_save", "> Save Level", FileRequest::Save),
    ("file_save_as", "> Save Level As", FileRequest::SaveAs),
    ("file_close", "> Close Level", FileRequest::CloseLevel),
    ("project_new", "> New Project", FileRequest::NewProject),
    ("project_open", "> Open Project", FileRequest::OpenProject),
];

/// A command that queues one request.
struct FileCommand {
    id: &'static str,
    label: &'static str,
    request: FileRequest,
    queue: FileRequestQueue,
}

impl EditorCommand for FileCommand {
    fn id(&self) -> &str {
        self.id
    }

    fn label(&self) -> &str {
        self.label
    }

    fn execute(&self, _ctx: &EditorContext) {
        self.queue.push(self.request);
    }
}

/// Registers the seven file and project commands; each one queues its request in `queue`.
pub fn register_file_commands(registry: &mut CommandRegistry, queue: &FileRequestQueue) {
    for (id, label, request) in FILE_COMMANDS {
        registry.register(FileCommand {
            id,
            label,
            request,
            queue: queue.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_reflect::TypeRegistry;

    #[test]
    fn file_save_command_queues_a_save_request() {
        let queue = FileRequestQueue::new();
        let mut ctx = EditorContext::new(TypeRegistry::new());
        register_file_commands(&mut ctx.commands, &queue);

        assert!(ctx.commands.run(&ctx, "file_save"));
        assert_eq!(queue.take_all(), vec![FileRequest::Save]);
        assert!(queue.take_all().is_empty());
    }

    #[test]
    fn every_file_command_is_registered_once() {
        let queue = FileRequestQueue::new();
        let mut registry = CommandRegistry::new();
        register_file_commands(&mut registry, &queue);
        register_file_commands(&mut registry, &queue);
        let ids = registry.ids();
        assert_eq!(ids.len(), FILE_COMMANDS.len());
        for (id, _, _) in FILE_COMMANDS {
            assert!(registry.contains(id), "missing {id}");
        }
    }
}
