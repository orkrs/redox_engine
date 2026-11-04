# AST-01 — Импорт glTF в движок: материалы, текстуры, LOD, конвейер

- **Статус:** не начат.
- **Размер:** 5 шагов (S, M, M, M, S) ≈ 5–7 дней. **Крейты:** `redox_asset`, `redox_render` (шаг 3), `redox_cook` (шаг 4). **Зависит от:** MAT-08 шаг 2–3 (флаги материала) для шага 3. **Блокирует:** ничего.
- **Модель:** haiku по шагам. Шаг 3 касается рендерера: выполнять в окно «писателя рендерера» (`../FOREST_WAVE.md` §3).
- Источник: `../FOREST_SCENE_FINDINGS.md`, E11, E12, A1–A7; план 5.4.

## Суть
Пример `forest` грузит `.glb` собственным загрузчиком на крейте `gltf` (`examples/forest/model.rs`): привязывает встроенные текстуры к материалам, запекает `KHR_texture_transform` в UV, считает тангенты, вырезает 16-бит, достраивает UV стволам, дублирует листья. Нужно перенести всё полезное в движок, чтобы любой glTF из Blender, Poly Haven или Sketchfab грузился штатно и через `redox_cook` попадал в `.pak` без перекодирования PNG.

## Что уже есть (проверено чтением)
- `redox_asset/src/loader/gltf_loader.rs`: `load_scene_from_path/bytes`, `assemble_scene`, `read_primitive`, `convert_material`, `pixels_to_rgba8` (уже конвертирует 16-бит через `narrow16`), `compute_tangents`, `decompose_trs`. Данные: `GltfMeshData` (positions, normals, uvs, tangents, indices; **нет индекса материала**), `GltfMaterialData` (цвет, металличность, шероховатость, путь внешней текстуры базового цвета), `GltfNodeData`, `GltfTextureData`, `GltfSceneData`.
- `redox_render/src/asset_types.rs`: `MaterialData` (`with_albedo_ktx2`, `with_normal`, `with_mr`), `renderer/assets.rs`: `add_material_from_asset`, `add_mesh_from_asset`; потоковые текстуры: `resource/texture_stream.rs`.
- `redox_cook`: команды `scan`, `pack`, `list`, `verify`, `bake_meshlets`, `bake_sdf` (**ПРОВЕРИТЬ** `crates/redox_cook/src/`).
- Образец решений: `crates/redox_core/examples/forest/model.rs` (материал → текстуры, `uv_transform`, `trunk_uvs`, `encode_png`).
- Известные особенности экспорта Blender 5.2 из Poly Haven: `"texCoord":-1` рядом с `KHR_texture_transform`; `extensionsRequired: KHR_texture_transform`; нормали в 16-бит PNG; альфа склеена с цветом в одну RGBA-картинку; у стволов нет UV; все LOD в одном файле (имена `*_LOD0`, `*_LOD1`).

## Шаг 1 — расследование (S)
Отчёт `../notes/AST-01-step1.md`:
- таблица «что в файле → что теряется в `GltfSceneData` → что нужно движку» по полям: индекс материала у примитива, `baseColor/normal/metallicRoughness/occlusion/emissive` текстуры и их `texCoord`, `KHR_texture_transform`, `alphaMode`/`alphaCutoff`/`doubleSided`, `emissiveFactor`, `COLOR_0`, `TEXCOORD_1`, расширения (`KHR_materials_emissive_strength`, `_transmission`, `_ior`, `_specular`, `EXT_mesh_gpu_instancing`, `MSFT_lod`);
- проверка, какие тесты загрузчика уже есть (`crates/redox_asset/tests` и `#[cfg(test)]` в файле);
- проверка, поддерживает ли `gltf` крейт нужные features (`KHR_texture_transform`, `KHR_materials_*`, `extensions`) в корневом `Cargo.toml` (`gltf = "1.4"`), и как они включаются без побочных эффектов для остальных крейтов;
- есть ли в окружении кодировщик BC7 (`cargo search` / реестр; запись KTX2 в `redox_cook`): результат нужен шагу 4.

## Шаг 2 — расширение данных и терпимость к Blender (M)
- `GltfMeshData.material: Option<usize>`; `GltfMaterialData`: `base_color/normal/metallic_roughness/occlusion/emissive: Option<TextureRef { image_index: usize, tex_coord: u32, transform: Option<UvTransform> }>`, `alpha_mode: AlphaMode`, `alpha_cutoff: f32`, `double_sided: bool`, `emissive_factor: [f32;3]`; вершинные цвета `colors: Vec<[f32;4]>` и `uvs1` — опционально.
- Нормализатор JSON-чанка GLB: заменяет `"texCoord":-1` на `"texCoord": 0` (длина строки та же); применяется перед `gltf::import_slice`, только когда парсинг упал или всегда (выбор записать); тест на фикстуре с реальным шаблоном ошибки.
- Достройка UV, если у примитива нет `TEXCOORD_0`: цилиндрическая проекция вокруг Y (как `trunk_uvs`), помечается флагом `generated_uv` в данных меша; тест.
- Тесты (фикстуры `.glb`, собираемые в тесте из `gltf-json` или бинарных строк): `a_material_gets_its_own_textures`, `texture_transform_is_reported`, `sixteen_bit_normals_are_converted`, `alpha_modes_and_double_sided_are_read`, `a_blender_texcoord_minus_one_still_loads`, `a_mesh_without_uvs_gets_generated_ones`.

## Шаг 3 — мост к рендереру (M; окно писателя рендерера; нужен MAT-08 шаг 2–3)
- `redox_render/src/asset_types.rs`: `MaterialData::from_gltf(&GltfMaterialData, &textures) -> MaterialData`; `renderer/assets.rs`: загрузка встроенных текстур материала (sRGB для базового цвета и emissive, линейные для нормали и metal/rough) и выставление `texture_index`, `normal_texture_index`, `mr_texture_index`, `AlphaMode`, `double_sided`.
- UV-преобразование запекается в вершины при импорте (опция `bake_uv_transform: true` по умолчанию), чтобы шейдер не менялся; в `GltfMeshData` остаются исходные UV отдельно, если нужна обратимость (записать решение).
- Тесты (GPU): `a_gltf_primitive_draws_with_its_textures` (маленький куб-фикстура с известной цветной текстурой: пиксель на грани ≈ цвет текстуры ± допуск), `a_blend_material_is_masked_and_double_sided`.

## Шаг 4 — конвейер `redox_cook import-gltf` (M)
- Новая команда: `redox_cook import-gltf <in.glb> <out_dir|.pak> [--max-texture 2048] [--lods]`: меш (`.bin` по формату `redox_asset`), материал (ron), текстуры в KTX2 с полной цепочкой mip. Если шаг 1 нашёл кодировщик BC7, использовать BC7 (sRGB для цвета, линейный для нормали/MR); иначе KTX2 RGBA8 с mip и исход 📌 «BC7 нет, нужен кодировщик».
- Выделение LOD: объекты с суффиксом `_LOD<N>` собираются в `LodGroup { levels: Vec<MeshRef> }` с порогами по экранному размеру из `--lods` (по умолчанию: `LOD0` до 25 % экрана, `LOD1` до 8 %, `LOD2` дальше; значения можно переопределить).
- Тесты: `import_gltf_roundtrips_into_a_pak`, `lod_groups_are_formed_from_name_suffixes`, `textures_are_written_with_mips`.
- Скрипт экспорта Blender (выбор объектов, смещение в начало координат, уменьшение текстур) положить в `scripts/blender_export_pick.py`; описать в `docs/ASSETS.md` (версия Blender 5.2, известные особенности).

## Шаг 5 — пример на API движка (S)
- `examples/forest`: удалить `model.rs`, грузить через `redox_asset` и мост шага 3; кадр `--shot` на тех же ассетах совпадает с прежним в допуске 3/255 (если `forest` ещё в рабочем виде).
- Обновить `FOREST_SCENE_FINDINGS.md` (E11, E12 → статусы).

## Границы
- **Читать:** `gltf_loader.rs`, `asset_types.rs`, `renderer/assets.rs`, `resource/texture_stream.rs`, `redox_cook/src`, `examples/forest/model.rs`.
- **Создать:** фикстуры в тестах, `scripts/blender_export_pick.py`, `docs/engineering/notes/AST-01-step1.md`.
- **Менять:** `redox_asset/src/loader/gltf_loader.rs`, `redox_render/src/{asset_types,renderer/assets}.rs`, `redox_cook`, `docs/ASSETS.md`, `examples/forest`.
- **Не трогать:** шейдеры, VG, GI.

## Проверка
`cargo fmt`, `cargo check -p redox_asset -p redox_cook -p redox_render --all-targets`; тесты своих крейтов по одному. Clippy и полный прогон — координатор. Крупные файлы в `assets/forest` в git не добавлять.

## Для исполнителя
См. `../FOREST_WAVE.md` §4. Шаг 3 только в согласованное окно; шаги 1, 2, 4, 5 безопасны параллельно с другими агентами (кроме правок `renderer/assets.rs`).

## Закрытие
✅ / ⛔ / 📌 по шагам; нехватка кодировщика BC7 это 📌 с числом (размер текстур RGBA8 против BC7 на ассетах `forest`).
