//! WebAssembly scripting for the RedOx Engine (see `docs/PLAN_SCRIPTING.md`, ADR 0005/0006).
//!
//! A [`ScriptRuntime`] hosts sandboxed script instances. Every call gets a fuel budget, memory
//! is capped, and host functions (`env.log`, `env.time_seconds`) are only linked when permitted.
//! A trap or fuel exhaustion disables just the offending instance.

#![warn(missing_docs)]

mod backend;
mod config;
mod error;
mod host;
mod runtime;
mod wasmi_backend;
#[cfg(feature = "wasmtime")]
mod wasmtime_backend;

pub use config::{Permissions, ScriptConfig};
pub use error::ScriptError;
pub use runtime::{BackendKind, ScriptRuntime};

/// Handle of an instance created by [`ScriptRuntime::instantiate`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InstanceId(pub u32);
