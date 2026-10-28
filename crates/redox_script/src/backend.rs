//! The private engine abstraction: a backend compiles modules into instances.

use crate::{ScriptCommand, ScriptError, ScriptEvent, WorldQueries, config::ScriptConfig};

/// A WebAssembly engine able to turn module bytes into runnable instances.
pub(crate) trait ScriptBackend {
    /// Compiles, links (honouring the permissions) and instantiates `wasm`.
    ///
    /// The module must export `on_update(f32)`.
    fn instantiate(
        &mut self,
        wasm: &[u8],
        config: &ScriptConfig,
    ) -> Result<Box<dyn BackendInstance>, ScriptError>;
}

/// One instantiated script, owned by the runtime.
pub(crate) trait BackendInstance {
    /// Adds `dt` to the clock and calls `on_update(dt)` with a fresh fuel budget.
    fn update(&mut self, dt: f32) -> Result<(), ScriptError>;
    /// Calls the exported `() -> i32` function `name` with a fresh fuel budget.
    fn call_i32(&mut self, name: &str) -> Result<i32, ScriptError>;
    /// Removes and returns the buffered log messages.
    fn take_logs(&mut self) -> Vec<String>;
    /// Reads an exported `f32` global.
    fn read_global_f32(&self, name: &str) -> Option<f32>;
    /// Sets a named input readable by the script.
    fn set_input(&mut self, name: &str, value: f32);
    /// Copies every input set so far (used when the module is reloaded).
    fn inputs(&self) -> Vec<(String, f32)>;
    /// Latest value the script stored under `name`.
    fn read_output(&self, name: &str) -> Option<f32>;
    /// Removes and returns the queued events.
    fn take_events(&mut self) -> Vec<ScriptEvent>;
    /// Removes and returns the queued commands.
    fn take_commands(&mut self) -> Vec<ScriptCommand>;
    /// Installs the world answering `env.raycast`.
    fn set_world_queries(&mut self, world: Box<dyn WorldQueries>);
}
