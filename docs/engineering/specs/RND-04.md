# RND-04 — Замер CPU-времени подготовки кадра (gate A1/A2)

- **Статус:** выполнено 2026-10-29, принято в `main`. Замер release: 3000 кубов ≈ 1,45 мс CPU (подготовка 0,03, запись 1,03, отправка 0,39).

- **Размер:** S. **Крейт:** `redox_render`. **Зависит от:** ничего. **Блокирует:** решение о кэше батчей (A2 «полное») и бюджет CPU ≤ 4 мс.
- **Модель:** haiku.

## Суть
В плане (`PLAN_EXECUTION.md`, A1/A2) числа «CPU до/после» не сняты: нет способа измерить, сколько CPU занимает `render_frame_into`
(сортировка/слияние батчей, запись команд, submit). Нужны счётчики, доступные и тестам, и F5-оверлею.

## Границы
- **Читать:** `renderer/frame.rs` (`render_frame_into`, строка 39; конец функции — где вызывается `queue.submit`), `renderer/mod.rs` (поле-счётчик по образцу `forward_draw_calls`),
  `renderer/accessors.rs` (как отданы геттеры), `tests/instancing.rs` (как собран кадр на 30 кубов).
- **Менять:** `renderer/frame.rs`, `renderer/mod.rs` (одно поле + `init.rs`/`from_shared_device`-инициализатор поля — найти `forward_draw_calls:` и добавить рядом),
  `renderer/accessors.rs` (геттер), новый `tests/frame_timings.rs`.
- **Не трогать:** `memory.rs` и оверлей (подключение к F5 — отдельная задача).

## Решение
```rust
/// CPU time spent in the last `render_frame_into`, in milliseconds (wall clock, not GPU time).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameCpuTimings {
    /// Visibility, batching and sorting of the object list.
    pub prepare_ms: f32,
    /// Recording every pass into command buffers.
    pub record_ms: f32,
    /// `queue.submit` and everything after it inside the call.
    pub submit_ms: f32,
}
impl FrameCpuTimings { pub fn total_ms(&self) -> f32 { self.prepare_ms + self.record_ms + self.submit_ms } }
impl Renderer { pub fn last_frame_cpu_timings(&self) -> FrameCpuTimings; }
```
Реэкспорт из `lib.rs` — **не** делать (владелец добавит; в тестах путь `redox_render::renderer::FrameCpuTimings`, проверить видимость модуля; если `renderer` приватен — в отчёте написать).
Измерение через `std::time::Instant`: `t0` в начале `render_frame_into`; `t1` после `batching::permute` (до `begin_frame_state`); `t2` перед `queue.submit`; `t3` в конце. Поле
`last_cpu_timings: FrameCpuTimings` записывается в конце функции (в том числе при раннем `Ok`). Ранний возврат с ошибкой — поле не меняется.
Никаких аллокаций и логирования в горячем пути.

## Пошагово
1. Тест → красный (не компилируется; допустимо `todo!()` в геттере для этого шага).
2. Реализация.
3. В отчёте приложить число: `cargo test -p redox_render --test frame_timings --release -- --nocapture` печатает `prepare/record/submit` для 30 и 3000 одинаковых кубов.

## Тест (`tests/frame_timings.rs`; сцена и кадр — как в `tests/instancing.rs`, пропуск без адаптера)
```rust
#[test]
fn timings_are_recorded_and_split_into_three_parts() {
    // render 3 frames of 30 cubes, then:
    let t = renderer.last_frame_cpu_timings();
    assert!(t.total_ms() > 0.0);
    assert!(t.prepare_ms >= 0.0 && t.record_ms > 0.0 && t.submit_ms >= 0.0);
    assert!(t.total_ms() < 1000.0, "sanity: a frame of 30 cubes is not a second");
}
#[test]
fn more_objects_do_not_make_the_prepare_step_cheaper() {
    // 3000 objects vs 30 (median of 5 frames each): prepare(3000) >= prepare(30). No absolute limits (machine dependent).
}
```

## Проверка
`cargo test -p redox_render --test frame_timings`, `cargo test -p redox_render --test instancing --test golden_frames` (кадры не изменились), clippy.

## Для исполнителя
`git merge --ff-only main`. Результат измерения (числа в release) впиши в отчёт — я перенесу его в `PLAN_EXECUTION.md` (A1/A2).
