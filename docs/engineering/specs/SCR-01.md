# SCR-01 — Горячая перезагрузка WASM-модуля (`ScriptRuntime::reload`)

- **Размер:** S. **Крейт:** `redox_script`. **Зависит от:** ничего. **Блокирует:** SCR-02 (удобство SDK, не жёстко).
- **Модель:** haiku.

## Суть
Чтобы править скрипт без перезапуска игры, нужно заменить модуль у существующего `InstanceId`. Сейчас `ScriptRuntime` умеет только
`instantiate` (новый id) — все ссылки игры на id теряются. Выбор владельца в плане: горячая перезагрузка входит в «полное» B7, но не в минимум.

## Границы
- **Читать:** `runtime.rs` (`Slot`, `instantiate`, `set_input`, `slot_mut`), `backend.rs` (трейты `ScriptBackend`, `BackendInstance`), `host.rs` (поле `inputs: HashMap<String,f32>` в `HostData`, строка 104),
  `wasmi_backend.rs`, `wasmtime_backend.rs` (структуры экземпляров и где лежит `HostData`).
- **Менять:** `runtime.rs`, `backend.rs` (+1 метод в `BackendInstance`), `wasmi_backend.rs`, `wasmtime_backend.rs`, `host.rs` (метод выгрузки входов),
  новый `tests/reload.rs`.
- **Не трогать:** `config.rs`, `lib.rs`, права API.

## Решение
```rust
impl ScriptRuntime {
    /// Replaces the module of `id` with `wasm`. The inputs set so far are kept; outputs, queued events/commands,
    /// logs, globals and the clock start over. If the new module does not compile, link or lacks `on_update`, the old
    /// module keeps running and the error is returned. A disabled (trapped) instance becomes enabled again on success.
    pub fn reload(&mut self, id: InstanceId, wasm: &[u8]) -> Result<(), ScriptError>;
}
```
Хранение входов: в `BackendInstance` добавить `fn inputs(&self) -> Vec<(String, f32)>` (копия `HostData.inputs`; реализация в обоих бэкендах — по одной строке
через общий метод `HostData::inputs_snapshot`). `reload`:
1. `slot_mut(id)` → нет → `Err(ScriptError::Link("unknown instance".into()))` (проверить, какой вариант ошибки подходит; не добавлять новый, если есть подходящий).
2. `inputs = slot.instance.inputs()`.
3. `new = self.backend.instantiate(wasm, &self.config)?` (старый экземпляр не тронут при ошибке).
4. Для каждого входа `new.set_input(..)`; если был `world`, он **не** переносится (описать в doc-комментарии: `set_world_queries` вызвать заново).
5. `slot.instance = new; slot.disabled = false`.

## Тесты (`tests/reload.rs`; `wasm()`, `backends()`, `runtime()` — копия из `tests/host_commands.rs`)
```rust
const V1: &str = r#"(module (import "env" "get_f32" (func $g (param i32 i32) (result f32)))
  (import "env" "set_f32" (func $s (param i32 i32 f32))) (memory (export "memory") 1)
  (data (i32.const 0) "inout")
  (func (export "on_update") (param f32) (call $s (i32.const 2) (i32.const 3)
     (f32.add (call $g (i32.const 0) (i32.const 2)) (f32.const 1)))))"#;  // out = in + 1
const V2: &str = /* same, but + 100 */;
#[test] fn reload_swaps_the_logic_and_keeps_the_inputs() { /* set in=5; update -> out 6; reload(V2); update -> out 105 (input kept) ; both backends */ }
#[test] fn a_broken_module_leaves_the_old_one_running() { /* reload(&[1,2,3]) is Err; update still gives the V1 result */ }
#[test] fn a_module_without_on_update_is_rejected_and_the_old_one_stays() { /* (module) */ }
#[test] fn reload_revives_a_trapped_instance() { /* module with (unreachable) in on_update traps -> is_disabled; reload(V1) -> !is_disabled and works */ }
#[test] fn reload_of_an_unknown_instance_is_an_error() { /* InstanceId(9) */ }
#[test] fn reload_drops_queued_events() { /* module emits an event; update; reload(V1); take_events is empty */ }
```
(Тела писать полностью; WAT для `V2`, ловушки и модуля с событием — по образцу `host_commands.rs`.)

## Проверка
`cargo test -p redox_script`, `cargo test -p redox_script --features wasmtime`, clippy (оба набора фич).

## Для исполнителя
`git merge --ff-only main`. Не добавляй наблюдение за файлами — это отдельная задача (SCR-03 в бэклоге).
