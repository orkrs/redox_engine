//! WebAssembly scripting for the RedOx Engine (see `docs/PLAN_SCRIPTING.md`, ADR 0005/0006).
//!
//! A [`ScriptRuntime`] hosts sandboxed script instances. Every call gets a fuel budget, memory
//! is capped, and host functions (`env.log`, `env.time_seconds`, `env.get_f32`, `env.set_f32`, `env.emit_event`, `env.emit_command`, `env.raycast`, `env.raycast_many`) are only linked when permitted.
//! A trap or fuel exhaustion disables just the offending instance.

#![warn(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

#[cfg(test)]
mod api_tests;
mod backend;
mod config;
mod error;
mod host;
mod runtime;
mod wasmi_backend;
#[cfg(feature = "wasmtime")]
mod wasmtime_backend;
mod watch;
mod world;

pub use config::{ApiPermissions, Permissions, ScriptConfig};
pub use error::ScriptError;
pub use runtime::{BackendKind, ScriptRuntime};
pub use watch::{ScriptWatcher, WatchEvent};
pub use world::WorldQueries;

/// A named event emitted by a script with `env.emit_event`.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptEvent {
    /// Event name (at most 64 bytes of UTF-8).
    pub name: String,
    /// Payload value.
    pub value: f32,
}

/// A command emitted by a script with `env.emit_command`: a name and three numbers.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptCommand {
    /// Command name (at most 64 bytes of UTF-8).
    pub name: String,
    /// The three arguments, in the order the script passed them.
    pub args: [f32; 3],
}

/// Handle of an instance created by [`ScriptRuntime::instantiate`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InstanceId(pub u32);
