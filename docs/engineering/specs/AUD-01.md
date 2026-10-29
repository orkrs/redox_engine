# AUD-01 — Музыка из файла на диске (потоковое чтение)

- **Статус:** выполнено 2026-10-29, принято в `main`.

- **Размер:** S. **Крейт:** `redox_audio`. **Зависит от:** ничего. **Блокирует:** ничего.
- **Модель:** haiku.

## Суть
`register_music(name, bytes)` (`music_play.rs:56`) держит файл целиком в памяти (`Arc<[u8]>` в `music_sources`, `context.rs:71`).
Для длинных треков (5–10 МБ Ogg) это лишняя память. Нужен вариант «зарегистрировать путь, читать с диска при проигрывании».

## Границы
- **Читать:** `music_play.rs` (целиком: `open_stream`, `register_music`, `start_music_voice`), `context.rs` (поле `music_sources`, строки 71 и 141),
  `tests/music_streaming.rs`, `tests/data/tone.ogg`; в исходниках kira 0.12 — `StreamingSoundData::from_file`
  (`~/.cargo/registry/src/*/kira-0.12*/src/sound/streaming/sound_data.rs`).
- **Менять:** `music_play.rs`, `context.rs` (тип поля), `tests/music_streaming.rs`.
- **Не трогать:** `music_apply.rs`, `cues.rs`, шины.

## Решение
```rust
/// Where the encoded bytes of a music track come from.
pub(crate) enum MusicSource {
    /// The whole file, in memory.
    Memory(Arc<[u8]>),
    /// A file read from disk while the track plays.
    File(PathBuf),
}
impl AudioContext {
    /// Registers a music file by path (Ogg, MP3, FLAC, WAV). The header is checked now; the file is streamed from disk when played.
    pub fn register_music_file(&mut self, name: impl Into<String>, path: impl Into<PathBuf>) -> Result<(), MusicError>;
}
```
Поле: `music_sources: HashMap<String, MusicSource>`. `register_music` кладёт `Memory`. `has_music` как раньше. `start_music_voice`: `Memory` → `StreamingSoundData::from_cursor`,
`File` → `StreamingSoundData::from_file(path)`; ошибка открытия — `log::warn!` и выход, как сейчас. Проверка заголовка в `register_music_file`: открыть `from_file`, отбросить;
ошибка → `MusicError` (посмотреть варианты в `music_play.rs:26`; если подходящего нет, добавить `Io(String)`).
Файл читается по требованию: файл может быть удалён между регистрацией и запуском — это `log::warn!`, а не паника.

## Тесты (`tests/music_streaming.rs`; без устройства тесты используют проверки логики, как соседние — посмотреть, как они обходят отсутствие звукового устройства)
```rust
#[test] fn a_file_track_registers_and_is_known() {
    // path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/tone.ogg")
    // ctx.register_music_file("tone", path).unwrap(); assert!(ctx.has_music("tone"));
}
#[test] fn a_missing_file_is_rejected_with_an_error() { /* register_music_file("x", "no/such.ogg").is_err(), has_music("x") == false */ }
#[test] fn a_text_file_is_rejected_as_not_audio() { /* write a temp file with "hello"; is_err() */ }
#[test] fn a_file_track_starts_a_streaming_voice_when_a_device_exists() {
    // like the existing memory test: skip (return) when !ctx.is_available(); play_music("tone"); update_music(0.0);
    // assert_eq!(ctx.playing_music_voices(), 1)
}
```

## Проверка
`cargo test -p redox_audio`, clippy.

## Для исполнителя
`git merge --ff-only main`. Не меняй поведение `register_music`: все прежние тесты зелёные без правок.
