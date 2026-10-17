//! `wasmi` implementation of the script backend (interpreter, fuel metered).

use wasmi::{
    Caller, Config, CustomFuelCosts, Engine, Extern, Instance, Linker, Module, Store, StoreLimits,
    StoreLimitsBuilder, TrapCode, TypedFunc, Val,
};

use crate::{
    ScriptError,
    backend::{BackendInstance, ScriptBackend},
    config::{Permissions, ScriptConfig},
    host::HostData,
};

/// Name of the entry point every script must export.
const UPDATE_EXPORT: &str = "on_update";
/// Name of the memory export host functions read from.
const MEMORY_EXPORT: &str = "memory";

/// Bytes moved by bulk memory operations per unit of fuel.
const BYTES_COPIED_PER_FUEL: u32 = 1024;

/// Store data: the backend independent host state plus the wasmi resource limits.
struct State {
    host: HostData,
    limits: StoreLimits,
}

/// Compiles modules with a shared fuel-metered `wasmi` engine.
pub(crate) struct WasmiBackend {
    engine: Engine,
}

impl WasmiBackend {
    pub(crate) fn new() -> Self {
        let mut config = Config::default();
        config.consume_fuel(true);
        // Bulk memory operations (`memory.grow`, `memory.fill`, ...) are metered by bytes
        // copied. The default (64 bytes per fuel) would make growing by 1000 pages cost more
        // than the default per-call budget, so bytes are metered more coarsely. The memory
        // limit still bounds the work done by a single operation.
        config.fuel_cost(CustomFuelCosts {
            bytes_copied_per_fuel: BYTES_COPIED_PER_FUEL,
            fuel_per_bytes_translated: 7,
            fuel_per_bytes_validated: 2,
        });
        Self {
            engine: Engine::new(&config),
        }
    }
}

impl ScriptBackend for WasmiBackend {
    fn instantiate(
        &mut self,
        wasm: &[u8],
        config: &ScriptConfig,
    ) -> Result<Box<dyn BackendInstance>, ScriptError> {
        let module =
            Module::new(&self.engine, wasm).map_err(|e| ScriptError::Compile(e.to_string()))?;
        let linker = build_linker(&self.engine, config.permissions)?;

        let state = State {
            host: HostData::default(),
            limits: StoreLimitsBuilder::new()
                .memory_size(config.memory_limit_bytes)
                .build(),
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|state| &mut state.limits);
        // The start function runs during instantiation and needs fuel as well.
        store
            .set_fuel(config.fuel_per_call)
            .map_err(|e| ScriptError::Link(e.to_string()))?;

        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| ScriptError::Link(e.to_string()))?;
        let on_update = instance
            .get_typed_func::<f32, ()>(&store, UPDATE_EXPORT)
            .map_err(|_| ScriptError::MissingExport(UPDATE_EXPORT.to_string()))?;

        Ok(Box::new(WasmiInstance {
            store,
            instance,
            on_update,
            fuel_per_call: config.fuel_per_call,
        }))
    }
}

/// Defines only the host functions the permissions allow; everything else fails to link.
fn build_linker(engine: &Engine, permissions: Permissions) -> Result<Linker<State>, ScriptError> {
    let link_err = |e: wasmi::errors::LinkerError| ScriptError::Link(e.to_string());
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

/// A live `wasmi` instance with its own store.
struct WasmiInstance {
    store: Store<State>,
    instance: Instance,
    on_update: TypedFunc<f32, ()>,
    fuel_per_call: u64,
}

impl WasmiInstance {
    fn refuel(&mut self) -> Result<(), ScriptError> {
        self.store
            .set_fuel(self.fuel_per_call)
            .map_err(|e| ScriptError::Trap(e.to_string()))
    }
}

impl BackendInstance for WasmiInstance {
    fn update(&mut self, dt: f32) -> Result<(), ScriptError> {
        self.store.data_mut().host.clock += dt;
        self.refuel()?;
        self.on_update
            .call(&mut self.store, dt)
            .map_err(map_call_error)
    }

    fn call_i32(&mut self, name: &str) -> Result<i32, ScriptError> {
        let func = self
            .instance
            .get_typed_func::<(), i32>(&self.store, name)
            .map_err(|_| ScriptError::MissingExport(name.to_string()))?;
        self.refuel()?;
        func.call(&mut self.store, ()).map_err(map_call_error)
    }

    fn take_logs(&mut self) -> Vec<String> {
        self.store.data_mut().host.take_logs()
    }

    fn read_global_f32(&self, name: &str) -> Option<f32> {
        match self
            .instance
            .get_global(&self.store, name)?
            .get(&self.store)
        {
            Val::F32(value) => Some(value.to_float()),
            _ => None,
        }
    }
}

fn map_call_error(error: wasmi::Error) -> ScriptError {
    match error.as_trap_code() {
        Some(TrapCode::OutOfFuel) => ScriptError::OutOfFuel,
        _ => ScriptError::Trap(error.to_string()),
    }
}
