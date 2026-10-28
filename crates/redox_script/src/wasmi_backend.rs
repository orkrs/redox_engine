//! `wasmi` implementation of the script backend (interpreter, fuel metered).

use wasmi::{
    Caller, Config, CustomFuelCosts, Engine, Extern, Instance, Linker, Module, Store, StoreLimits,
    StoreLimitsBuilder, TrapCode, TypedFunc, Val,
};

use crate::WorldQueries;
use crate::{
    ScriptCommand, ScriptError, ScriptEvent,
    backend::{BackendInstance, ScriptBackend},
    config::ScriptConfig,
    host::{self, HostData},
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
        let linker = build_linker(&self.engine, config)?;

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
fn build_linker(engine: &Engine, config: &ScriptConfig) -> Result<Linker<State>, ScriptError> {
    let link_err = |e: wasmi::errors::LinkerError| ScriptError::Link(e.to_string());
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
        linker
            .func_wrap("env", "emit_command", host_emit_command)
            .map_err(link_err)?;
    }
    if api.world {
        linker
            .func_wrap("env", "raycast", host_raycast)
            .map_err(link_err)?;
        linker
            .func_wrap("env", "raycast_many", host_raycast_many)
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

/// `env.emit_command(name_ptr, name_len, a, b, c)`: invalid names are ignored.
fn host_emit_command(mut caller: Caller<'_, State>, ptr: i32, len: i32, a: f32, b: f32, c: f32) {
    if let Some(name) = read_name(&mut caller, ptr, len) {
        caller.data_mut().host.emit_command(&name, [a, b, c]);
    }
}

/// `env.raycast(ox, oy, oz, dx, dy, dz, max_distance) -> f32`: the hit distance or -1.0.
fn host_raycast(
    mut caller: Caller<'_, State>,
    ox: f32,
    oy: f32,
    oz: f32,
    dx: f32,
    dy: f32,
    dz: f32,
    max_distance: f32,
) -> f32 {
    caller
        .data_mut()
        .host
        .raycast([ox, oy, oz], [dx, dy, dz], max_distance)
}

/// `env.raycast_many(rays_ptr, count, out_ptr) -> i32`: answers a batch of ray records.
///
/// Returns `count` when the batch was processed and 0 (writing nothing) on any invalid range.
fn host_raycast_many(
    mut caller: Caller<'_, State>,
    rays_ptr: i32,
    count: i32,
    out_ptr: i32,
) -> i32 {
    let Some(Extern::Memory(memory)) = caller.get_export(MEMORY_EXPORT) else {
        return 0;
    };
    let Some((rays, out)) =
        host::batch_ranges(memory.data(&caller).len(), rays_ptr, count, out_ptr)
    else {
        return 0;
    };
    let records = memory.data(&caller)[rays].to_vec();
    let Some(answers) = caller.data_mut().host.raycast_batch(&records) else {
        return 0;
    };
    memory.data_mut(&mut caller)[out].copy_from_slice(&host::answer_bytes(&answers));
    count
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
        self.store.data_mut().host.begin_call();
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
        self.store.data_mut().host.begin_call();
        self.refuel()?;
        func.call(&mut self.store, ()).map_err(map_call_error)
    }

    fn take_logs(&mut self) -> Vec<String> {
        self.store.data_mut().host.take_logs()
    }

    fn set_input(&mut self, name: &str, value: f32) {
        self.store.data_mut().host.set_input(name, value);
    }

    fn inputs(&self) -> Vec<(String, f32)> {
        self.store.data().host.inputs_snapshot()
    }

    fn read_output(&self, name: &str) -> Option<f32> {
        self.store.data().host.output(name)
    }

    fn take_events(&mut self) -> Vec<ScriptEvent> {
        self.store.data_mut().host.take_events()
    }

    fn take_commands(&mut self) -> Vec<ScriptCommand> {
        self.store.data_mut().host.take_commands()
    }

    fn set_world_queries(&mut self, world: Box<dyn WorldQueries>) {
        self.store.data_mut().host.set_world(world);
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
