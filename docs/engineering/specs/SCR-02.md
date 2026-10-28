# SCR-02 — SDK для авторов WASM-скриптов (`redox_script_sdk`)

- **Размер:** M. **Состояние:** **ЗАБЛОКИРОВАНО решением владельца** — на машине установлен только `x86_64-pc-windows-msvc`
  (`rustup target list --installed`), цели `wasm32-unknown-unknown` нет. Без неё нельзя собрать и проверить ни одного скрипта на Rust.
- **Что нужно от владельца:** разрешить `rustup target add wasm32-unknown-unknown` (≈ 20 МБ, меняет тулчейн) — или решить, что скрипты пишутся на WAT/C/AssemblyScript без SDK.
- **Зависит от:** SCR-01 (желательно). **Блокирует:** примеры скриптов для редактора.
- **Модель:** sonnet.

## Суть
Хост-API (`get_f32`, `set_f32`, `emit_event`, `emit_command`, `raycast`, `raycast_many`) — сырые импорты `env` с указателями. Автор на Rust должен писать
`input("px")`, `emit("coin", 3.0)`, а не `unsafe extern "C"`.

## Границы
- **Создать:** `crates/redox_script_sdk/` (отдельный крейт **вне** участников по умолчанию сборки хоста? — нет: обычный участник workspace, `#![no_std]`, `crate-type = ["rlib"]`),
  пример `crates/redox_script_sdk/examples/coins.rs` (cdylib через `[[example]] crate-type = ["cdylib"]`), тест `crates/redox_script/tests/sdk_example.rs`.
- **Читать:** `docs/PLAN_SCRIPTING.md`, `host.rs` (точные сигнатуры импортов, лимиты: имя ≤ 64 байт, 256 событий, 64 луча), `tests/host_world.rs`.
- **Не трогать:** хост (`redox_script/src`).

## Решение (набросок API, `no_std`)
```rust
pub fn input(name: &str) -> f32;                       // get_f32
pub fn output(name: &str, value: f32);                 // set_f32
pub fn emit(name: &str, value: f32);                   // emit_event
pub fn command(name: &str, a: f32, b: f32, c: f32);    // emit_command
pub fn raycast(origin: [f32; 3], dir: [f32; 3], max: f32) -> Option<f32>;   // raycast; кодирование «нет попадания» взять из host.rs
#[macro_export] macro_rules! script { ($update:ident) => { #[unsafe(no_mangle)] pub extern "C" fn on_update(dt: f32) { $update(dt) } } }
```
Правило: имена передаются как `(ptr, len)` строкового среза в линейной памяти модуля — безопасно, пока срез жив на время вызова.

## Пошагово (после разблокировки)
1. `rustup target add wasm32-unknown-unknown` (владелец).
2. Написать крейт и пример «монеты» (повторяет `scripted_coins.rs`), собрать `cargo build -p redox_script_sdk --example coins --target wasm32-unknown-unknown --release`.
3. Тест хоста: загрузить `.wasm` из `target/wasm32-unknown-unknown/release/examples/coins.wasm`, пропустить тест, если файла нет (CI без цели), иначе проверить событие `coin`.
4. Документ `docs/SCRIPTING_GUIDE.md` на одну страницу с примером.

## Для исполнителя
Не начинай, пока в каталоге задач нет отметки о разблокировке. Не устанавливай цель сам.
