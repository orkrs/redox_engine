# SKY-02 — Процедурная атмосфера, диск солнца, IBL из неба

- **Статус:** не начат. **Ворота:** после SKY-01 (учитывает его выводы).
- **Размер:** 5 шагов (M, M, M, S, S) ≈ 7–9 дней. **Крейт:** `redox_render`. **Зависит от:** SKY-01. **Блокирует:** ничего.
- **Модель:** haiku по шагам; шейдерная математика (шаги 1–2) вероятно требует доработки координатором. Один писатель рендерера одновременно (`../FOREST_WAVE.md` §3).
- Источник: `../FOREST_SCENE_FINDINGS.md`, E10; план 5.2.3–5.2.4.

## Суть
Сейчас небо это HDR-эквиректангулярная карта (она же даёт IBL), диска солнца нет, направление солнца с картой не связано. Нужна процедурная атмосфера по модели Hillaire 2020 («A Scalable and Production Ready Sky and Atmosphere Rendering Technique»): небо зависит от направления солнца, диск солнца виден, IBL берётся из неба, дальние объекты получают воздушную перспективу. Режим `Hdri` остаётся по умолчанию и не меняется.

## Что уже есть (проверено чтением)
- `pass/skybox.rs`, `shaders/skybox.wgsl` (полноэкранный треугольник), `renderer/settings.rs:239` (`set_skybox`), `resource/ibl.rs` (`IBLResource`, `environment_view`), `shaders/irradiance_convolution.wgsl`, `prefilter_convolution.wgsl`, `equirect_to_cube.wgsl`.
- Солнце: `DirectionalLight { direction (к солнцу), color, intensity, source_angle }` в `light.rs`.
- Туман: `post/fog_froxel.rs`, `shaders/fog_composite.wgsl`.

## Шаг 1 — таблицы пропускания и многократного рассеяния (M)
- `redox_render/src/sky.rs`, `shaders/sky_transmittance.wgsl`, `sky_multiscatter.wgsl`. Параметры: `AtmosphereParams { planet_radius_km, atmosphere_height_km, rayleigh_scattering, rayleigh_scale_height, mie_scattering, mie_absorption, mie_scale_height, mie_g, ozone_absorption, ground_albedo }` (значения Земли по умолчанию из статьи).
- CPU-эталон `transmittance_reference(params, height, cos_zenith)` (численное интегрирование, 256 шагов).
- Тесты (GPU): `transmittance_lut_matches_the_cpu_reference` (допуск 2 % по 64 выборкам), `transmittance_is_one_at_zero_distance`, `zenith_transmittance_is_greater_than_horizon`.

## Шаг 2 — sky-view и пасс неба (M; нужен шаг 1)
- `shaders/sky_view.wgsl` (LUT неба) и `sky.wgsl` (пасс), режим `SkyMode::{Hdri, Procedural}` (`renderer/settings.rs`, метод `set_sky_mode`); в режиме `Procedural` пасс заменяет `SkyboxPass`.
- Направление солнца берётся из направленного света кадра; в режиме `Hdri` кадры побитно прежние.
- Тесты (GPU): `a_low_sun_makes_the_horizon_redder_than_the_zenith` (солнце на 3° над горизонтом: R/B у горизонта в сторону солнца > R/B в зените), `the_zenith_is_bluer_at_noon`, `hdri_frames_are_bitwise_unchanged`.

## Шаг 3 — диск солнца и IBL из неба (M; нужен шаг 2)
- Диск: угловой размер из `source_angle`, яркость выше порога bloom (**ПРОВЕРИТЬ** порог в `post/bloom.rs`), с ослаблением атмосферой.
- IBL: небо рендерится в куб 128² раз при смене направления солнца или параметров (не каждый кадр; гистерезис по углу ≥ 1°), дальше существующие свёртки (`irradiance_convolution.wgsl`, `prefilter_convolution.wgsl`); `IBLResource` обновляется так же, как при `set_environment`.
- Тесты: `the_sun_disk_is_the_brightest_pixel_in_the_frame`, `ibl_colour_follows_the_sky` (средний цвет irradiance сверху ближе к цвету неба в зените, чем к серому), `moving_the_sun_by_less_than_one_degree_does_not_rebuild_the_cube`.

## Шаг 4 — воздушная перспектива (S; нужен шаг 2)
- Мягкое подмешивание рассеяния вместо плоского цвета тумана: в `fog_composite.wgsl` цвет тумана при `SkyMode::Procedural` берётся из sky-view по направлению луча. Плотность задаёт существующий `FogSettings`.
- Тест: `distant_geometry_takes_the_sky_colour` (дальний куб на 300 м ближе по цвету к небу, чем такой же ближний).
- Поправки по выводам SKY-01 (B) для лучей света: добавить сюда, если расследование нашло причину в настройках срезов.

## Шаг 5 — замер, пример, документы (S)
- Замер GPU-времени неба и пересборки куба на 1080p; цель: небо ≤ 0,5 мс, пересборка куба ≤ 2 мс. Если не вышло: 📌 с числом.
- `examples/forest`: режим `FOREST_SKY=procedural` включает небо; обновить `FOREST_SCENE_FINDINGS.md` (E10 → ✅/📌).
- `docs/` справка по шейдерам, `CHANGELOG.md`.

## Границы
- **Читать:** `pass/skybox.rs`, `shaders/{skybox,fog_composite,bloom}.wgsl`, `resource/ibl.rs`, `renderer/settings.rs`, `light.rs`, `frame_graph.rs`.
- **Создать:** `src/sky.rs`, `shaders/sky_*.wgsl`, `tests/sky_gpu.rs`.
- **Менять:** `lib.rs`, `renderer/{settings,frame,passes}.rs`, `frame_graph.rs`, `fog_composite.wgsl`.
- **Не трогать:** GI, VG, материалы, тени.

## Проверка
`cargo fmt`, `cargo check -p redox_render --all-targets`; свои тесты по одному. Clippy, полный прогон, замеры — координатор.

## Для исполнителя
См. `../FOREST_WAVE.md` §4. Один шаг за заход; формулы бери из статьи Hillaire и сверяй с CPU-эталоном. Если LUT не сходится с эталоном за две попытки, остановись и отчитайся с измеренной ошибкой.

## Закрытие
✅ / ⛔ / 📌 по шагам. Режим `Hdri` побитно прежний, иначе шаг не закрыт.
