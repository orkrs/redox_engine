//! `wasmtime` implementation of the script backend (Cranelift JIT, fuel metered).
//!
//! Only compiled with the `wasmtime` cargo feature; used to compare numbers with `wasmi`.

use std::sync::Mutex;

use wasmtime::{
    Caller, Config, Engine, Extern, Instance, Linker, Module, Store, StoreLimits,
    StoreLimitsBuilder, Trap, TypedFunc, Val,
};

use crate::{
    ScriptError, ScriptEvent,
    backend::{BackendInstance, ScriptBackend},
    config::ScriptConfig,
    host::{self, HostData},
};

/// Name of the entry point every script must export.
const UPDATE_EXPORT: &str = "on_update";
/// Name of the memory export host functions read from.
const MEMORY_EXPORT: &str = "memory";

/// Store data: the backend independent host state plus the wasmtime resource limits.
struct State {
    host: HostData,
    limits: StoreLimits,
}

/// Compiles modules with a shared fuel-metered `wasmtime` engine.
pub(crate) struct WasmtimeBackend {
    engine: Engine,
}

impl WasmtimeBackend {
    pub(crate) fn new() -> Result<Self, ScriptError> {
        let mut config = Config::new();
        config.consume_fuel(true);
        let engine = Engine::new(&config).map_err(|e| ScriptError::Compile(format!("{e:#}")))?;
        Ok(Self { engine })
    }
}

impl ScriptBackend for WasmtimeBackend {
    fn instantiate(
        &mut self,
        wasm: &[u8],
        config: &ScriptConfig,
    ) -> Result<Box<dyn BackendInstance>, ScriptError> {
        let module =
            Module::new(&self.engine, wasm).map_err(|e| ScriptError::Compile(format!("{e:#}")))?;
        let linker = build_linker(&self.engine, config)?;

        let state = State {
            host: HostData::default(),
            limits: StoreLimitsBuilder::new()
                .memory_size(config.memory_limit_bytes)
                .build(),
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(config.fuel_per_call)
            .map_err(|e| ScriptError::Link(format!("{e:#}")))?;

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| ScriptError::Link(format!("{e:#}")))?;
        let on_update = instance
            .get_typed_func::<f32, ()>(&mut store, UPDATE_EXPORT)
            .map_err(|_| ScriptError::MissingExport(UPDATE_EXPORT.to_string()))?;

        Ok(Box::new(WasmtimeInstance {
            store: Mutex::new(store),
            instance,
            on_update,
            fuel_per_call: config.fuel_per_call,
        }))
    }
}

/// Defines only the host functions the permissions allow; everything else fails to link.
fn build_linker(engine: &Engine, config: &ScriptConfig) -> Result<Linker<State>, ScriptError> {
    let link_err = |e: wasmtime::Error| ScriptError::Link(format!("{e:#}"));
    let (permissions, api) = (config.permissions, config.api);
    let mut linker = Linker::new(engine);
    if permissions.log {
        linker.func_wrap("env", "log", host_log).map_err(link_err)?;
    }
    if permissions.time {
        linker
            .func_wrap("env", "time_seconds", |caller: Caller<'_, State>| {
                caller.data().host.clock
            })
            .map_err(link_err)?;
    }
    if api.state {
        linker
            .func_wrap("env", "get_f32", host_get_f32)
            .map_err(link_err)?;
        linker
            .func_wrap("env", "set_f32", host_set_f32)
            .map_err(link_err)?;
    }
    if api.events {
        linker
            .func_wrap("env", "emit_event", host_emit_event)
            .map_err(link_err)?;
    }
    Ok(linker)
}

/// `env.log(ptr, len)`: copies a bounded slice of guest memory into the log buffer.
///
/// Out-of-bounds ranges are ignored (and reported through the `log` crate) instead of trapping.
fn host_log(mut caller: Caller<'_, State>, ptr: i32, len: i32) {
    let Some(Extern::Memory(memory)) = caller.get_export(MEMORY_EXPORT) else {
        log::warn!("script called log without exporting `{MEMORY_EXPORT}`");
        return;
    };
    let range = u32::try_from(ptr)
        .ok()
        .zip(u32::try_from(len).ok())
        .and_then(|(start, len)| {
            let start = usize::try_from(start).ok()?;
            Some(start..start.checked_add(usize::try_from(len).ok()?)?)
        });
    let bytes = range.and_then(|range| memory.data(&caller).get(range).map(<[u8]>::to_vec));
    match bytes {
        Some(bytes) => caller.data_mut().host.push_log(&bytes),
        None => log::warn!("script passed an out-of-bounds range to log ({ptr}, {len})"),
    }
}

/// Reads a script supplied name from the guest memory export; `None` when invalid.
fn read_name(caller: &mut Caller<'_, State>, ptr: i32, len: i32) -> Option<String> {
    let Some(Extern::Memory(memory)) = caller.get_export(MEMORY_EXPORT) else {
        return None;
    };
    host::read_name(memory.data(&*caller), ptr, len).map(str::to_owned)
}

/// `env.get_f32(name_ptr, name_len) -> f32`: 0.0 for unknown or invalid names.
fn host_get_f32(mut caller: Caller<'_, State>, ptr: i32, len: i32) -> f32 {
    match read_name(&mut caller, ptr, len) {
        Some(name) => caller.data().host.input(&name),
        None => 0.0,
    }
}

/// `env.set_f32(name_ptr, name_len, value)`: invalid names are ignored.
fn host_set_f32(mut caller: Caller<'_, State>, ptr: i32, len: i32, value: f32) {
    if let Some(name) = read_name(&mut caller, ptr, len) {
        caller.data_mut().host.set_output(&name, value);
    }
}

/// `env.emit_event(name_ptr, name_len, value)`: invalid names are ignored.
fn host_emit_event(mut caller: Caller<'_, State>, ptr: i32, len: i32, value: f32) {
    if let Some(name) = read_name(&mut caller, ptr, len) {
        caller.data_mut().host.emit(&name, value);
    }
}

/// A live `wasmtime` instance with its own store.
///
/// The store sits behind a mutex only because reading a global needs `&mut Store` in wasmtime
/// while the runtime API reads globals through `&self`.
struct WasmtimeInstance {
    store: Mutex<Store<State>>,
    instance: Instance,
    on_update: TypedFunc<f32, ()>,
    fuel_per_call: u64,
}

/// Exclusive access to a store behind a mutex; a poisoned lock is still usable here.
fn store_mut(store: &mut Mutex<Store<State>>) -> &mut Store<State> {
    store
        .get_mut()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl BackendInstance for WasmtimeInstance {
    fn update(&mut self, dt: f32) -> Result<(), ScriptError> {
        let store = store_mut(&mut self.store);
        store.data_mut().host.clock += dt;
        store
            .set_fuel(self.fuel_per_call)
            .map_err(|e| ScriptError::Trap(format!("{e:#}")))?;
        self.on_update.call(store, dt).map_err(map_call_error)
    }

    fn call_i32(&mut self, name: &str) -> Result<i32, ScriptError> {
        let store = store_mut(&mut self.store);
        let func = self
            .instance
            .get_typed_func::<(), i32>(&mut *store, name)
            .map_err(|_| ScriptError::MissingExport(name.to_string()))?;
        store
            .set_fuel(self.fuel_per_call)
            .map_err(|e| ScriptError::Trap(format!("{e:#}")))?;
        func.call(store, ()).map_err(map_call_error)
    }

    fn take_logs(&mut self) -> Vec<String> {
        store_mut(&mut self.store).data_mut().host.take_logs()
    }

    fn set_input(&mut self, name: &str, value: f32) {
        store_mut(&mut self.store)
            .data_mut()
            .host
            .set_input(name, value);
    }

    fn read_output(&self, name: &str) -> Option<f32> {
        self.store.lock().ok()?.data().host.output(name)
    }

    fn take_events(&mut self) -> Vec<ScriptEvent> {
        store_mut(&mut self.store).data_mut().host.take_events()
    }

    fn read_global_f32(&self, name: &str) -> Option<f32> {
        let mut store = self.store.lock().ok()?;
        let global = self.instance.get_global(&mut *store, name)?;
        match global.get(&mut *store) {
            Val::F32(bits) => Some(f32::from_bits(bits)),
            _ => None,
        }
    }
}

fn map_call_error(error: wasmtime::Error) -> ScriptError {
    match error.downcast_ref::<Trap>() {
        Some(Trap::OutOfFuel) => ScriptError::OutOfFuel,
        _ => ScriptError::Trap(format!("{error:#}")),
    }
}
