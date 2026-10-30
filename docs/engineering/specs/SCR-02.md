# SCR-02 — SDK для авторов WASM-скриптов (`redox_script_sdk`)

- **Статус:** не начат. **Одобрено владельцем 2026-10-29** (в общем «да» на оставшиеся пункты); цель `wasm32-unknown-unknown` устанавливает **координатор** (`rustup target add wasm32-unknown-unknown`, ≈ 20 МБ) перед запуском агента; агент цель не ставит.
- **Размер:** M. На машине до установки только `x86_64-pc-windows-msvc` (`rustup target list --installed`).
- **Решение владельца при отказе:** если цель ставить нельзя — скрипты на WAT/C/AssemblyScript без SDK, спека закрывается.
- **Зависит от:** SCR-01 (готов); **SCR-04** (после него SDK покрывает и новое хост-API: `find_by_name`, `overlap_*`, `nav_path`, `sys.*`-команды — шаг 5 ниже). **Блокирует:** примеры скриптов для редактора.
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
5. После SCR-04: обёртки `find_by_name(&str) -> Option<EntityId>`, `find_by_tag`, `position_of`, `overlap_sphere`, `get_value_of`, `nav_path`, `system_command(...)` с тестом `sdk_guard_example` (пример сторожа из SCR-04 шаг 5, собранный из Rust).

## Проверка
`cargo fmt`, `cargo clippy -p redox_script_sdk -p redox_script --all-targets -- -D warnings`, `cargo build -p redox_script_sdk --example coins --target wasm32-unknown-unknown --release`, `cargo test -p redox_script --test sdk_example`, `cargo test -p redox_script --features wasmtime --test sdk_example`, `cargo check --workspace --all-targets`, `python scripts/check_file_sizes.py`, `python scripts/gen_crates_doc.py --check`.

## Для исполнителя
Не начинай, пока в каталоге задач нет отметки о разблокировке. Не устанавливай цель сам.

## Закрытие (финальная волна)

Исход обязателен: ✅ / ⛔ / 📌 (`../FINAL_WAVE.md` §1). Агенту разрешён `cargo check -p <крейт> --all-targets` в своём worktree; тесты, clippy и замеры — у координатора. Шаг, у которого приёмочный тест после доработки красный, не остаётся «частично»: агент сообщает измеренное число, исход назначает координатор.

- **Подготовка координатором:** `rustup target add wasm32-unknown-unknown` до запуска агента.
- **Приёмка:** `redox_script_sdk` собирается под `wasm32-unknown-unknown`, пример-скрипт выполняется в `redox_script` и вызывает `sys.*` из SCR-04.
