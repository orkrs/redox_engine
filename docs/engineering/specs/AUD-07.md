# AUD-07 — Авторский инструментарий для звука: проверка cue-библиотек, громкость, «звуковая лаборатория»

- **Статус:** шаги 1–2 и 4 выполнены 2026-10-29 (валидатор, анализ громкости, `redox_cook audio-check`, документы); шаг 3 частично: `LabModel` и `--selftest` есть, окно egui с вкладками Cue/Музыка/3D/Анализ не сделано. Эталон LUFS — по RMS (спека ошибалась), тишина = −120 дБ.

- **Размер:** 4 шага (S, M, M, S) ≈ 7 дней. **Крейты:** `redox_audio`, `redox_cook`, `redox_core` (пример/панель). **Зависит от:** AUD-01, AUD-05, AUD-06 (готовы). **Блокирует:** —.
- **Модель:** haiku по шагам 1, 2, 4; шаг 3 (UI-панель) — haiku с проверкой координатора.
- Объём — «минимум»: звукорежиссёр без кода может (1) проверить библиотеку cue и музыку на ошибки, (2) увидеть громкость и обрезание, (3) послушать cue, слои адаптивной музыки и 3D-эмиттер в отдельном приложении. **Не входит:** DAW-подобный редактор, запись, микширование с графом эффектов (AUD-02 исключён), интеграция в `redox_editor` (панель пишется как автономный пример на `egui` — переносимая).

## Суть и аудит
- Есть: `SoundCue`/`CueLibrary` (RON, `cues.rs`), приоритеты и вытеснение, шины, музыка с диска (AUD-01), `AdaptiveMusic` (AUD-06), эмиттеры с окклюзией (AUD-05), `redox_audio::debug` (**ПРОВЕРИТЬ** что там), загрузчик (`loader.rs`).
- Нет: валидации библиотеки до запуска игры (опечатка в имени файла всплывает в логе во время игры), измерения громкости (LUFS/пик) и предупреждения «пик > 0 dBFS», проверки зацикливания музыки (шов в конце), инструмента прослушивания.

## Шаг 1 — валидатор cue-библиотек и ассетов звука (S, `redox_audio`)
- `redox_audio/src/validate.rs`:
```rust
pub enum Severity { Error, Warning }
pub struct Finding { pub severity: Severity, pub cue: Option<String>, pub message: String }
/// Checks a parsed library against the files available through `exists` / `read`.
pub fn validate_cue_library(lib: &CueLibrary, exists: &dyn Fn(&str) -> bool) -> Vec<Finding>;
```
Правила: `Error` — файл варианта не найден; пустой список вариантов; `volume`/`pitch` вне допустимых диапазонов; `cooldown < 0`; дубликат имени cue; приоритет вне диапазона; цикл в `chain` (если есть); `Warning` — вариант один при `random_pitch == 0` (монотонно); `cooldown == 0` у часто вызываемого cue (`tags` содержит `"footstep"`); два cue с одним и тем же набором файлов.
Тесты: по одному на правило (`a_missing_file_is_an_error`, `a_duplicate_cue_name_is_an_error`, …) + `a_valid_library_has_no_findings` + `all_findings_are_reported_not_only_the_first`.

## Шаг 2 — анализ файлов и команда `redox_cook audio-check` (M)
- `redox_audio/src/analyze.rs`: `analyze_samples(samples: &[f32], channels: u16, rate: u32) -> AudioStats { peak_db, rms_db, lufs_integrated (BS.1770, K-весовые фильтры + гейтинг 400 мс), duration, clipped_samples, dc_offset }`; `loop_seam_error(samples, channels, window) -> f32` (разность средних амплитуд в окне вокруг стыка начала/конца, в дБ; для музыки с `loop = true`).
- Декодирование — существующим загрузчиком/декодером (Symphonia через kira? **ПРОВЕРИТЬ** что доступно без новых зависимостей: `redox_audio::loader`), никаких новых крейтов без записи в отчёте.
- `redox_cook audio-check <assets-dir> [--cues cues.ron] [--target-lufs -16] [--tolerance 3]`: валидация шага 1 + для каждого файла: `peak > -0.1 dBFS` → Warning «клиппинг», LUFS вне `target ± tolerance` → Warning (для музыки и эффектов — отдельные цели: `--target-lufs-sfx -18`), шов зацикливания > 3 дБ → Warning, длительность эффектов > 10 с → Warning; код выхода 1, если есть Error; `--json` печатает отчёт машинно.
Тесты (синтетика в памяти): `a_full_scale_sine_peaks_at_zero_dbfs`, `a_minus_20_dbfs_sine_has_the_expected_rms` (−23,0 дБ ±0,1), `lufs_of_a_997hz_sine_at_minus_23_dbfs_is_minus_23_lufs_within_0_2` (эталонное значение BS.1770: синус 997 Гц, −23 дБFS, mono = −23,0 LUFS ±0,2 для одного канала — **сверить с формулой**, допуск расширить до ±0,5 при необходимости с записью причины), `silence_is_flagged`, `clipped_samples_are_counted`, `a_perfect_loop_has_a_near_zero_seam_error`, `a_hard_cut_has_a_large_seam_error`, `the_json_report_roundtrips`.

## Шаг 3 — «Звуковая лаборатория»: пример с панелью (M, `redox_core`)
Пример `crates/redox_core/examples/audio_lab.rs` (egui, как другие примеры):
- Список cue из загруженной библиотеки (`--cues cues.ron`; перезагрузка по кнопке и по изменению файла, `ScriptWatcher`-подобный опрос), кнопка «▶» играет cue (все варианты по очереди и «случайный»), индикаторы: вариант, громкость/питч, cooldown, приоритет, активные голоса и вытеснение.
- Вкладка «Музыка»: список слоёв `AdaptiveMusic` (ползунки параметров, `approach`), переключатель треков, показ положения.
- Вкладка «3D»: источник на плоскости, перетаскиваемый мышью вокруг слушателя; показывает расстояние, затухание, окклюзию (стены — простые боксы в тестовой сцене физики).
- Вкладка «Анализ»: результаты `audio-check` по папке (таблица, красные/жёлтые строки), без повторного декодирования при открытии (кэш на время сессии).
- Режим автопроверки `--selftest` (без окна и без устройства вывода): грузит тестовую библиотеку, играет cue на «пустом» контексте, проверяет счётчики голосов; выход 0/1 (аналог `foxrun --shot`).
Тесты (без окна): `the_selftest_of_audio_lab_passes_without_an_output_device`, `reloading_a_changed_library_updates_the_list` (логика списка — отдельная структура `LabModel`), `the_listener_distance_matches_the_dragged_position` (чистая математика панели 3D).

## Шаг 4 — документация (S)
`docs/AUDIO_AUTHORING.md` (как писать `cues.ron`, цели громкости, как запускать `audio-check` и лабораторию, примеры вывода), `PLAN_AUDIO_UI.md` (раздел инструментов), `PLAN_EXECUTION.md` (B8: «авторский инструментарий»), `CHANGELOG.md`, `docs/CRATES.md`.

## Уточнения по отчёту исполнителя (2026-10-29)
1. **Аксессор разрешён:** в `redox_audio/src/cues.rs` добавить `pub fn cues(&self) -> impl Iterator<Item = &SoundCue>` (файл добавлен в «Менять»; больше ничего в `cues.rs` не менять).
2. **Правила шага 1, которые `CueLibrary::parse` уже проверяет** (дубликат имени, пустое имя, перевёрнутые диапазоны, NaN, отрицательный cooldown), в `validate_cue_library` **не дублировать**. Вместо этого добавить `validate_cue_ron(text: &str, exists: &dyn Fn(&str) -> bool) -> Vec<Finding>`: ошибка `CueLibrary::parse` превращается в `Finding { severity: Error, cue: None/имя из ошибки, message: текст ошибки }`, при успехе — остальные правила на `lib.cues()` (файл не найден, пустой список вариантов, предупреждения). Тесты на «дубликат» и «cooldown < 0» идут через `validate_cue_ron` (вход — RON-текст), остальные — через `validate_cue_library`.
3. **Моно/стерео:** `StaticSoundData` всегда стерео (`Frame { left, right }`). Считать файл моно, если `left == right` во всех кадрах (точное равенство `f32`); тогда LUFS/RMS считаются по одному каналу (иначе моно «громче» на ~3 дБ). Запись «mono» — в `AudioStats.channels_detected`. Тест: `a_duplicated_mono_signal_is_analysed_as_mono`.
4. `Cargo.lock` в worktree не нужен (cargo не запускается).

## Границы
- **Читать:** `redox_audio/src/{cues,loader,music,music_layers,spatial,debug}.rs`, `redox_cook/src/{main,nav}.rs` (форма команд), `redox_core/examples/courier/*` (образец egui в примере), `redox_ui`.
- **Создать:** `redox_audio/src/{validate,analyze}.rs`, `redox_cook/src/audio_check.rs`, `redox_core/examples/audio_lab.rs`, тесты.
- **Менять:** `redox_audio/src/lib.rs`, `redox_audio/src/cues.rs` (только аксессор `cues()`), `redox_cook/src/main.rs` (общий файл — один агент).
- **Не трогать:** воспроизведение, шины, `kira`-обёртки.

## Проверка
`cargo fmt`, `cargo clippy -p redox_audio -p redox_cook -p redox_core --all-targets -- -D warnings`, `cargo test -p redox_audio`, `cargo test -p redox_cook`, `cargo run -p redox_core --example audio_lab --release -- --selftest`, `cargo check --workspace --all-targets`, `python scripts/check_file_sizes.py`, `python scripts/gen_crates_doc.py --check`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Новых зависимостей не добавлять; если декодер недоступен для анализа без нового крейта — остановись и опиши. `redox_cook/src/main.rs` правит один агент.

## Закрытие (финальная волна)

Исход обязателен: ✅ / ⛔ / 📌 (`../FINAL_WAVE.md` §1). Агенту разрешён `cargo check -p <крейт> --all-targets` в своём worktree; тесты, clippy и замеры — у координатора. Шаг, у которого приёмочный тест после доработки красный, не остаётся «частично»: агент сообщает измеренное число, исход назначает координатор.

- **Осталось (шаг 3):** окно egui `audio_lab` с вкладками Cue/Музыка/3D/Анализ поверх готового `LabModel`.
- **Приёмка:** `--selftest` зелёный, модель окна проверена тестом без GPU; живую проверку делает владелец (QA-01).
- **Исход: ✅ с оговорками** (2026-10-30): окно `audio_lab` с вкладками Cue/Музыка/3D/Анализ, 13 тестов модели зелёные. Не сделано (мелочи): кнопки «все варианты по очереди/случайно» и индикаторы голосов на Cue, стены-боксы на вкладке 3D. Пороги анализа продублированы из `redox_cook audio-check` (общий дом в `redox_audio` — пожелание).
