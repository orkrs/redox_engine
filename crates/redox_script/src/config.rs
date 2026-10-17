//! Runtime configuration: permissions, memory limit and fuel budget.

/// Default per-instance linear memory limit (16 MiB).
const DEFAULT_MEMORY_LIMIT_BYTES: usize = 16 * 1024 * 1024;
/// Default fuel budget granted to every call into a script.
const DEFAULT_FUEL_PER_CALL: u64 = 1_000_000;

/// Host capabilities a script may import from the `"env"` module.
///
/// Importing a host function whose permission is not granted fails at instantiation with
/// [`ScriptError::Link`](crate::ScriptError::Link).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Permissions {
    /// Allows importing `env.log(ptr, len)`.
    pub log: bool,
    /// Allows importing `env.time_seconds() -> f32`.
    pub time: bool,
}

impl Permissions {
    /// Every host function is permitted.
    pub const fn all() -> Self {
        Self {
            log: true,
            time: true,
        }
    }

    /// No host function is permitted; only self-contained modules can be instantiated.
    pub const fn none() -> Self {
        Self {
            log: false,
            time: false,
        }
    }
}

/// Host API v1a functions a script may import (`get_f32`, `set_f32`, `emit_event`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ApiPermissions {
    /// Allows importing `env.get_f32` and `env.set_f32`.
    pub state: bool,
    /// Allows importing `env.emit_event`.
    pub events: bool,
}

impl ApiPermissions {
    /// Every API function is permitted.
    pub const fn all() -> Self {
        Self {
            state: true,
            events: true,
        }
    }

    /// No API function is permitted.
    pub const fn none() -> Self {
        Self {
            state: false,
            events: false,
        }
    }
}

/// Limits and permissions applied to every instance of a [`ScriptRuntime`](crate::ScriptRuntime).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScriptConfig {
    /// Maximum size of a guest linear memory in bytes. Growth beyond it makes `memory.grow`
    /// return -1; a module whose initial memory exceeds it fails to instantiate.
    pub memory_limit_bytes: usize,
    /// Fuel granted at the start of every call (`update`, `call_i32`). Running out disables
    /// the instance.
    pub fuel_per_call: u64,
    /// Host functions the scripts may import.
    pub permissions: Permissions,
    /// Host API v1a functions the scripts may import.
    pub api: ApiPermissions,
}

impl Default for ScriptConfig {
    /// 16 MiB of memory, 1,000,000 fuel per call and all permissions granted.
    fn default() -> Self {
        Self {
            memory_limit_bytes: DEFAULT_MEMORY_LIMIT_BYTES,
            fuel_per_call: DEFAULT_FUEL_PER_CALL,
            permissions: Permissions::all(),
            api: ApiPermissions::all(),
        }
    }
}
