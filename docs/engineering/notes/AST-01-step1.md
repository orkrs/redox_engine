# AST-01 — шаг 1: расследование glTF-пути

Дата: 2026-11-04. Ветка: `worktree-agent-a6b48674ebffe3876`. Код не менялся.

## 1. Что в файле → что теряется → что нужно движку

Загрузчик: `crates/redox_asset/src/loader/gltf_loader.rs` (639 строк). Данные: `GltfSceneData`.

| В файле glTF | Что сейчас делает загрузчик | Что нужно движку (шаг 2 / 3) |
|---|---|---|
| `primitive.material` (индекс материала) | Теряется: `GltfMeshData` не хранит индекс | `GltfMeshData.material: Option<usize>` (шаг 2) |
| `baseColorTexture` (texture → image, `texCoord`) | Только внешний URI в `base_color_texture_path`; встроенные картинки лежат отдельно в `textures` по позиции, связь с материалом потеряна | `TextureRef { image_index, tex_coord, transform }` для base color (шаг 2) |
| `normalTexture` | Теряется полностью | `TextureRef` для нормали (шаг 2) |
| `metallicRoughnessTexture` | Теряется полностью | `TextureRef` (шаг 2) |
| `occlusionTexture`, `emissiveTexture` | Теряются | `TextureRef` (шаг 2; рендерер пока не использует) |
| `KHR_texture_transform` | Теряется; требует feature `KHR_texture_transform` у крейта `gltf` | `UvTransform` в `TextureRef`; запекание в UV на шаге 3 |
| `alphaMode`, `alphaCutoff`, `doubleSided` | Теряются | `AlphaMode`, `alpha_cutoff`, `double_sided` (шаг 2) |
| `emissiveFactor` | Теряется | `emissive_factor: [f32;3]` (шаг 2) |
| `COLOR_0` | Теряется | `colors: Vec<[f32;4]>` (шаг 2, опционально по спеке) |
| `TEXCOORD_1` | Теряется | `uvs1` (шаг 2, опционально по спеке) |
| `TEXCOORD_0` отсутствует | Нули (`[0,0]`) | Цилиндрические UV вокруг Y, флаг `generated_uv` (шаг 2) |
| `TANGENT` / вычисление | Есть, Lengyel (локальная копия) | Без изменений |
| `KHR_materials_ior`, `KHR_materials_specular` | Не читаются | Не нужны рендереру по forest; пропустить |
| `KHR_materials_emissive_strength`, `_transmission`, `EXT_mesh_gpu_instancing`, `MSFT_lod` | Не читаются | В forest-файлах не встречаются (см. раздел 3) |
| Текстуры 16 бит | `pixels_to_rgba8` сворачивает через `narrow16` | Без изменений (тест на нормали, шаг 2) |
| Узлы, LOD-объекты | Узлы плоские с `mesh_index`; `*_LOD` в имени узла | Группировка LOD на шаге 4 |

Связанный факт для шага 3: в `crates/redox_core/examples/forest/model.rs` `alpha_mode() != Opaque` считает BLEND тоже маской (alpha test). В forest-файлах листья имеют `BLEND` (`fir`, `sapling`, `grass2`). Это поведение примера, а не стандарта. Решение по BLEND/MASK переносится в шаг 3 и тест `a_blend_material_is_masked_and_double_sided`.

## 2. Тесты загрузчика, что уже есть

Файл `crates/redox_asset/src/loader/gltf_loader/tests.rs` (266 строк), 6 тестов:
`extensions_include_gltf_and_glb`, `loads_triangle_gltf_from_temp_file`, `computes_tangents_from_uvs`, `loads_glb_container`, `loads_gltf_with_external_png`; плюс `assert_triangle_payload`. Фикстуры собираются в тестах (base64, `wrap_glb`). Это готовый образец для новых фикстур шага 2.

## 3. Проверка на реальных файлах (только чтение, в репозиторий не добавлено)

Скрипт: `C:\Users\yann1n\AppData\Local\Temp\claude\3f7c0d1712cd1c53\ea830a79-3409-4e19-a140-ddd9ff1bba96\scratchpad\glb_survey.py` и `glb_facts.py`. Прочитаны все 45 файлов `assets/forest/models/*.glb`.

- `"texCoord": -1` в JSON: **0 вхождений во всех 45 файлах.** Спека описывает этот дефект (экспорт Blender 5.2), но в текущих файлах он не воспроизводится. Нормализатор всё равно нужен по спеке (шаг 2), но как запасной путь (см. решение в отчёте).
- `KHR_texture_transform`: в 15 файлах (fir_*, sapling_*, их `_lod`); `extensionsRequired` содержит его же. Используется только в `baseColorTexture`.
- `KHR_materials_ior`, `KHR_materials_specular`: в растительности; других расширений материалов нет. `EXT_mesh_gpu_instancing`, `MSFT_lod`: не найдены.
- Альфа: `BLEND` у листьев и трав (fir, sapling, grass2), `OPAQUE` у остальных. `doubleSided` стоит у всех листовых материалов.
- `TEXCOORD_0` отсутствует только у одного примитива (ствол) в каждом fir_*; у остальных файлов UV есть у всех примитивов.
- `COLOR_0`: у fir_* (4 примитива на файл). Остальные файлы без вершинных цветов.
- Текстуры: все встроенные PNG. Разрядность пикселей обзором не проверялась (только размеры из заголовков PNG). Размеры: 1024² у растительности, grass и fern, 2048² у камней, земли и горы.
- LOD: отдельные узлы с `_LOD` в имени есть в fir_a, boulder (1 узел) и в `*_lod.glb`-файлах; то есть LOD часто лежит в одном файле.
- Размер RGBA8 с mip-цепочкой (расчёт `w*h*4*4/3` по PNG-заголовкам) на один материал:
  - 1024²: около 5,3 МБ RGBA8 с mip; BC7 с mip (1 байт на texel) около 1,33 МБ.
  - 2048²: около 21,3 МБ RGBA8 с mip; BC7 около 5,3 МБ.
  - `boulder.glb`: две PNG-карты 2048² дают около 44,7 МБ RGBA8 с mip (расчёт учёл только PNG-заголовки; третья картинка не PNG и не учтена). Тот же набор в BC7 с mip: около 11,2 МБ, то есть в четыре раза меньше.

## 4. Feature `gltf` крейта

- Корневой `Cargo.toml` строка 68: `gltf = "1.4"`. Версия в lock: gltf 1.4.1.
- Feature `KHR_texture_transform` только включает `gltf-json/KHR_texture_transform`; без новых зависимостей. Метод `texture::Info::texture_transform()` доступен только с ней. У `NormalTexture`, `OcclusionTexture` такого метода нет (проверено по исходникам 1.4.1), поэтому для нормали и occlusion трансформ не читается.
- `redox_core` уже включает её: `gltf = { workspace = true, features = ["KHR_texture_transform"] }`. Значит, включение в workspace не меняет набор фич остальных крейтов. Изменение безопасно.
- Для шага 2: `gltf::import` (путь) и `import_slice` (байты), а также публичные `import_buffers` и `import_images` и `Gltf::from_slice`. Это позволяет повторно импортировать нормализованный JSON для `.gltf` с внешними буферами.

## 5. BC7-кодировщик

- В `Cargo.lock` нет ни одного BC-кодировщика (`intel_tex`, `texpresso`, `ctt-*`, `texture-compressor` отсутствуют). В локальном реестре `~/.cargo/registry` тоже.
- `redox_asset` с feature `encode` использует `basis-universal` (UASTC/ETC1S). Это не BC7.
- `cargo search bc7` (сеть) показал кандидатов: `texture-compressor` 0.2.3 (BC1-BC7, по описанию детерминированный), `ctt-*` (ISPC/C++ биндинги), `rusty_dds` 0.9.0 (BC1-7 кодирование в его описании). Ни один не проверен на сборку и лицензию.
- Вывод: **BC7 нет в workspace**. Шаг 4 делает KTX2 RGBA8 с mip (`redox_asset::Ktx2Texture` поддерживает `Rgba8Unorm` и `Rgba8Srgb`) и помечает 📌.

## 6. Расхождения со спекой

1. Спека: `.bin` меша «по формату redox_asset». В `redox_asset` **нет формата меша** (`.bin` меша нет; `GltfMeshData` только `serde`). Для шага 4 нужно решить формат. Решение описано в отчёте.
2. Спека: `"texCoord":-1` как дефект ассетов. В 45 файлах forest его нет (см. раздел 3). Нормализатор оставляем по спеке, но включаем только при ошибке импорта, чтобы не менять корректные файлы.
3. Спека не расходится по `redox_cook`, но для записи KTX2 RGBA8 удобнее `redox_asset::Ktx2Texture::new_2d(..., Rgba8Srgb | Rgba8Unorm, levels).to_ktx2_bytes(zstd)`: этот API уже есть, и `redox_cook` в зависимостях имеет `redox_asset` с feature `encode`.

## 7. Что проверить координатору

- Выбор формата меша для `.pak` (шаг 4) и BC7 как отдельная задача (кандидаты в разделе 5).
- Решение по BLEND против MASK (шаг 3).
