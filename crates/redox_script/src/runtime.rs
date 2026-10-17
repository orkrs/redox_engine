//! The public runtime: owns instances, delegates execution to a backend.

use crate::{
    InstanceId, ScriptConfig, ScriptError, ScriptEvent,
    backend::{BackendInstance, ScriptBackend},
    wasmi_backend::WasmiBackend,
};

struct Slot {
    instance: Box<dyn BackendInstance>,
    disabled: bool,
}

/// The WebAssembly engine behind a [`ScriptRuntime`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BackendKind {
    /// The `wasmi` interpreter (default).
    Wasmi,
    /// The `wasmtime` Cranelift JIT (cargo feature `wasmtime`).
    #[cfg(feature = "wasmtime")]
    Wasmtime,
}

/// Hosts script instances with fuel, memory and permission limits.
pub struct ScriptRuntime {
    config: ScriptConfig,
    backend: Box<dyn ScriptBackend>,
    slots: Vec<Slot>,
}

impl ScriptRuntime {
    /// Creates a runtime on the default (`wasmi`) backend that applies `config` to every instance.
    pub fn new(config: ScriptConfig) -> Self {
        Self {
            config,
            backend: Box::new(WasmiBackend::new()),
            slots: Vec::new(),
        }
    }

    /// Creates a runtime on an explicitly chosen engine.
    ///
    /// # Errors
    ///
    /// [`ScriptError::Compile`] if the engine cannot be created.
    pub fn with_backend(config: ScriptConfig, kind: BackendKind) -> Result<Self, ScriptError> {
        let backend: Box<dyn ScriptBackend> = match kind {
            BackendKind::Wasmi => Box::new(WasmiBackend::new()),
            #[cfg(feature = "wasmtime")]
            BackendKind::Wasmtime => Box::new(crate::wasmtime_backend::WasmtimeBackend::new()?),
        };
        Ok(Self {
            config,
            backend,
            slots: Vec::new(),
        })
    }

    /// Compiles, links and instantiates a module. It must export `on_update(f32)`.
    pub fn instantiate(&mut self, wasm: &[u8]) -> Result<InstanceId, ScriptError> {
        let id = u32::try_from(self.slots.len())
            .map_err(|_| ScriptError::Link("too many script instances".into()))?;
        let instance = self.backend.instantiate(wasm, &self.config)?;
        self.slots.push(Slot {
            instance,
            disabled: false,
        });
        Ok(InstanceId(id))
    }

    /// Reads `path` from an asset source and instantiates it.
    ///
    /// # Errors
    ///
    /// [`ScriptError::Asset`] if the file cannot be read, otherwise as [`Self::instantiate`].
    pub fn instantiate_from_source(
        &mut self,
        source: &dyn redox_asset::AssetSource,
        path: &str,
    ) -> Result<InstanceId, ScriptError> {
        let bytes = source
            .read(path)
            .map_err(|e| ScriptError::Asset(format!("{path}: {e}")))?;
        self.instantiate(&bytes)
    }

    /// Sets a named input value the script reads with `get_f32`; false for an unknown instance.
    pub fn set_input(&mut self, id: InstanceId, name: &str, value: f32) -> bool {
        match self.slot_mut(id) {
            Some(slot) => {
                slot.instance.set_input(name, value);
                true
            }
            None => false,
        }
    }

    /// The latest value the script stored under `name` with `set_f32`.
    pub fn read_output(&self, id: InstanceId, name: &str) -> Option<f32> {
        self.slot(id)?.instance.read_output(name)
    }

    /// Takes the events queued with `emit_event` since the last call, oldest first.
    pub fn take_events(&mut self, id: InstanceId) -> Vec<ScriptEvent> {
        self.slot_mut(id)
            .map(|slot| slot.instance.take_events())
            .unwrap_or_default()
    }

    /// Adds `dt` to the instance clock, then calls `on_update(dt)` with a fresh fuel budget.
    ///
    /// A trap or running out of fuel disables the instance.
    pub fn update(&mut self, id: InstanceId, dt: f32) -> Result<(), ScriptError> {
        let slot = self.live_slot(id)?;
        let result = slot.instance.update(dt);
        Self::disable_on_failure(slot, result)
    }

    /// Calls an exported function `() -> i32` with a fresh fuel budget.
    ///
    /// An unknown export is an error but does not disable the instance.
    pub fn call_i32(&mut self, id: InstanceId, name: &str) -> Result<i32, ScriptError> {
        let slot = self.live_slot(id)?;
        let result = slot.instance.call_i32(name);
        Self::disable_on_failure(slot, result)
    }

    /// Removes and returns the instance's buffered log messages (empty for unknown ids).
    pub fn take_logs(&mut self, id: InstanceId) -> Vec<String> {
        self.slot_mut(id)
            .map(|slot| slot.instance.take_logs())
            .unwrap_or_default()
    }

    /// Reads an exported `f32` global, if it exists.
    pub fn read_global_f32(&self, id: InstanceId, name: &str) -> Option<f32> {
        self.slot(id)?.instance.read_global_f32(name)
    }

    /// Whether the instance was disabled by a trap or fuel exhaustion.
    ///
    /// Unknown ids count as disabled.
    pub fn is_disabled(&self, id: InstanceId) -> bool {
        self.slot(id).is_none_or(|slot| slot.disabled)
    }

    fn slot(&self, id: InstanceId) -> Option<&Slot> {
        self.slots.get(usize::try_from(id.0).ok()?)
    }

    fn slot_mut(&mut self, id: InstanceId) -> Option<&mut Slot> {
        self.slots.get_mut(usize::try_from(id.0).ok()?)
    }

    fn live_slot(&mut self, id: InstanceId) -> Result<&mut Slot, ScriptError> {
        match self.slot_mut(id) {
            Some(slot) if !slot.disabled => Ok(slot),
            _ => Err(ScriptError::Disabled),
        }
    }

    fn disable_on_failure<T>(
        slot: &mut Slot,
        result: Result<T, ScriptError>,
    ) -> Result<T, ScriptError> {
        if matches!(result, Err(ScriptError::Trap(_) | ScriptError::OutOfFuel)) {
            slot.disabled = true;
        }
        result
    }
}

impl std::fmt::Debug for ScriptRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptRuntime")
            .field("config", &self.config)
            .field("instances", &self.slots.len())
            .finish()
    }
}
