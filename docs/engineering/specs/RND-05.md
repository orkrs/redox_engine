# RND-05 — `Renderer::try_new`: ошибка вместо паники при создании рендерера

- **Статус:** выполнено 2026-10-29, принято в `main`. Единственный вызывающий (`RenderContext::new`) оставлен на `new`: показать ошибку ему негде.

- **Размер:** S. **Крейт:** `redox_render` (+ вызывающие в `redox_core`). **Зависит от:** ничего. **Блокирует:** ничего.
- **Модель:** haiku.

## Суть
`Renderer::new(window, present_mode)` (`crates/redox_render/src/renderer/init.rs:11`) паникует через `expect`: «Failed to create surface»
(строка 22), «Failed to find a suitable GPU adapter» (33), «Failed to create device» (62). Игра на машине без Vulkan-драйвера
падает без сообщения пользователю. Библиотечный код должен возвращать ошибку (политика репозитория: `unwrap_used = "warn"`).

## Границы
- **Читать:** `init.rs` строки 1–80 (создание instance/surface/adapter/device), 4 места вызова `Renderer::new` (`grep -rn "Renderer::new" crates --include=*.rs`),
  `surface.rs` (образец типа ошибки `SurfaceError`).
- **Менять:** `init.rs`, при необходимости новый маленький модуль ошибки в `renderer/error.rs` (+ `mod error;` в `renderer/mod.rs`), вызывающие места (4 шт.).
- **Не трогать:** `from_shared_device` (используется тестами), остальной `init.rs` (строка 363 `legacy_texture(0).expect` остаётся: инвариант, регистр только что создан).

## Решение
```rust
/// Why a [`Renderer`] could not be created.
#[derive(Debug)]
pub enum RendererError {
    /// The window surface could not be created.
    Surface(String),
    /// No GPU adapter supports the surface and the required features.
    NoAdapter,
    /// The adapter refused the requested device.
    Device(String),
}
impl std::fmt::Display for RendererError { /* short English messages */ }
impl std::error::Error for RendererError {}

impl Renderer {
    pub async fn try_new(window: Arc<Window>, present_mode: wgpu::PresentMode) -> Result<Self, RendererError>;
    /// Panics with the error text if the renderer cannot be created; use [`Self::try_new`] to handle it.
    pub async fn new(window: Arc<Window>, present_mode: wgpu::PresentMode) -> Self {
        Self::try_new(window, present_mode).await.expect("renderer creation failed")
    }
}
```
Тело `try_new` — прежнее тело `new`, три `expect` заменяются на `map_err(...)?` / `ok_or(RendererError::NoAdapter)?`.
`RendererError` реэкспортировать в `lib.rs` рядом с `Renderer` (здесь `lib.rs` менять **можно**). Вызывающие места (примеры, редактор): оставить `new`, если окно создаётся в `resumed`
и показать ошибку нечем; иначе перевести на `try_new` и вывести `eprintln!` + корректный выход (`event_loop.exit()`), без `unwrap`.

## Пошагово
1. Вынести тело в `try_new`, `new` — тонкая обёртка.
2. Ошибка + `Display`.
3. Тест (ниже) и сборка всего workspace.

## Тесты
Создать настоящее окно в тесте нельзя, поэтому тестируется только ошибка и преобразование:
```rust
#[test]
fn renderer_error_messages_name_the_failing_step() {
    assert!(RendererError::NoAdapter.to_string().to_lowercase().contains("adapter"));
    assert!(RendererError::Surface("x".into()).to_string().contains("surface"));
    assert!(RendererError::Device("lost".into()).to_string().contains("lost"));
}
fn _assert_error_traits() { fn is<T: std::error::Error + Send + Sync + 'static>() {} is::<RendererError>(); }
```
(в `renderer/error.rs`, модуль `tests`). Ручная проверка владельцем: запуск Courier с переменной `WGPU_BACKEND=gl` (или без драйвера) даёт читаемое сообщение — пометить как «не проверено».

## Проверка
`cargo check --workspace --all-targets`, `cargo test -p redox_render --lib`, clippy `-D warnings`.

## Для исполнителя
`git merge --ff-only main`. Готово, когда в `init.rs` не осталось `expect` на surface/adapter/device, а workspace собирается.
