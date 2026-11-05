//! Editor commands addressed by identifier.

use crate::context::EditorContext;
use std::sync::Arc;

/// An action the editor runs by identifier (command palette, menu or hotkey).
pub trait EditorCommand: Send + Sync {
    /// Stable identifier, for example `create_entity`.
    fn id(&self) -> &str;
    /// Text shown in the command palette.
    fn label(&self) -> &str;
    /// Runs the command against the shared editor state.
    fn execute(&self, ctx: &EditorContext);
}

/// Commands by identifier, kept in registration order. Clones share the command objects.
#[derive(Clone, Default)]
pub struct CommandRegistry {
    commands: Vec<Arc<dyn EditorCommand>>,
}

impl CommandRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `command`. A command registered earlier with the same id is replaced.
    pub fn register(&mut self, command: impl EditorCommand + 'static) {
        let command: Arc<dyn EditorCommand> = Arc::new(command);
        let existing = self
            .commands
            .iter()
            .position(|known| known.id() == command.id());
        match existing {
            Some(index) => self.commands[index] = command,
            None => self.commands.push(command),
        }
    }

    /// Returns the command with identifier `id`, if registered.
    pub fn get(&self, id: &str) -> Option<&Arc<dyn EditorCommand>> {
        self.commands.iter().find(|command| command.id() == id)
    }

    /// Whether a command with identifier `id` is registered.
    pub fn contains(&self, id: &str) -> bool {
        self.get(id).is_some()
    }

    /// Identifiers of all commands, in registration order.
    pub fn ids(&self) -> Vec<&str> {
        self.commands.iter().map(|command| command.id()).collect()
    }

    /// Labels of all commands, in registration order.
    pub fn labels(&self) -> Vec<&str> {
        self.commands
            .iter()
            .map(|command| command.label())
            .collect()
    }

    /// `(id, label)` pairs of all commands, in registration order.
    pub fn entries(&self) -> Vec<(&str, &str)> {
        self.commands
            .iter()
            .map(|command| (command.id(), command.label()))
            .collect()
    }

    /// Runs the command `id` and returns `true`. For an unknown identifier it logs
    /// `[WARN] Unknown command: {id}` and returns `false`.
    pub fn run(&self, ctx: &EditorContext, id: &str) -> bool {
        match self.get(id) {
            Some(command) => {
                command.execute(ctx);
                true
            }
            None => {
                ctx.log.warn(format!("Unknown command: {id}"));
                false
            }
        }
    }
}
