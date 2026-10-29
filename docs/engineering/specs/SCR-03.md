# SCR-03 — Наблюдатель за файлом скрипта (автоматический `reload`)

- **Статус:** выполнено 2026-10-29, принято в `main`. Ошибка перезагрузки показывается через `Display`.

- **Размер:** S. **Крейт:** `redox_script`. **Зависит от:** SCR-01 (`ScriptRuntime::reload`). **Блокирует:** ничего.
- **Модель:** haiku.

## Суть
SCR-01 даёт `reload(id, wasm)`, но кто-то должен заметить, что файл `.wasm` изменился. Нужен маленький наблюдатель без новых зависимостей:
опрос времени изменения и размера файла.

## Границы
- **Читать:** `runtime.rs` (`reload` из SCR-01, `instantiate_from_source`), `lib.rs` (реэкспорты), любой тест с `std::env::temp_dir` в репозитории
  (`grep -rn "temp_dir" crates --include=*.rs | head`).
- **Создать:** `src/watch.rs`, `tests/watch.rs`. **Менять:** `src/lib.rs` (`mod watch; pub use watch::ScriptWatcher;`).
- **Не трогать:** остальные файлы. Новых зависимостей в `Cargo.toml` не добавлять (`notify` не нужен).

## Решение
```rust
/// Watches one `.wasm` file on disk and reloads the script instance when the file changes.
pub struct ScriptWatcher { /* path: PathBuf, id: InstanceId, last: Option<(SystemTime, u64)> */ }

/// What `poll` found.
#[derive(Debug, PartialEq)]
pub enum WatchEvent {
    /// The file is unchanged since the last poll (or does not exist yet and did not before).
    Unchanged,
    /// The file changed and the instance was reloaded.
    Reloaded,
    /// The file changed but reloading failed; the old module keeps running. Holds the error text.
    Failed(String),
}

impl ScriptWatcher {
    /// Starts watching `path` for `id`. The current state of the file is taken as "already loaded" (no reload on the first poll).
    pub fn new(path: impl Into<PathBuf>, id: InstanceId) -> Self;
    /// Looks at the file; if its modification time or size differ from the last poll, reads it and calls `runtime.reload(id, &bytes)`.
    /// A missing or unreadable file is `Unchanged` (editors save by delete-and-rename; the next poll sees the new file).
    /// A failed reload is reported once for that file state, not on every poll.
    pub fn poll(&mut self, runtime: &mut ScriptRuntime) -> WatchEvent;
}
```
Правила: сравнивать пару (время изменения, размер); при ошибке `metadata`/`read` — `Unchanged` и **не** обновлять `last`; после `Failed` запомнить новое состояние,
чтобы не повторять; никаких `unwrap`; игровой код вызывает `poll` раз в 0,5 с (в документации написать).

## Тесты (`tests/watch.rs`; файл создавать в `std::env::temp_dir()` с уникальным именем — pid + счётчик; WAT из `tests/reload.rs`)
```rust
#[test] fn the_first_poll_does_not_reload() { /* write V1; instantiate; watcher.poll == Unchanged */ }
#[test] fn a_changed_file_is_reloaded_and_runs_the_new_code() {
    // write V1 (out = in + 1); instantiate; watcher; write V2 (+100) (size differs or sleep 20 ms to change mtime; assert on polled result);
    // poll == Reloaded; set_input; update; read_output == in + 100
}
#[test] fn a_broken_file_fails_once_and_keeps_the_old_module() {
    // write garbage bytes; poll == Failed(..); poll again == Unchanged; update still gives the V1 result
}
#[test] fn a_missing_file_is_unchanged() { /* remove file; poll == Unchanged; no panic */ }
#[test] fn a_file_that_comes_back_after_deletion_is_reloaded() { /* remove, poll, write V2, poll == Reloaded */ }
```
Время изменения на Windows точно до 100 нс, но файловая система может округлять: для надёжности тесты делают файл V2 другого размера (добавить пробел/комментарий в WAT).

## Проверка
`cargo test -p redox_script --test watch`, `cargo test -p redox_script`, clippy, fmt.

## Для исполнителя
`git merge --ff-only main`; убедись, что SCR-01 уже в `main` (есть `ScriptRuntime::reload`). Иначе остановись.
