# EDT-B3 — отчёт исполнителя

- **Ветка:** `edt/b3`
- **Worktree:** `/home/besyak/Документы/redox/.worktrees/b3`
- **База:** `9afe43b` (main)
- **Cargo не запускался** (по заданию).

## Исход по шагам

| Шаг | Статус | Коммит(ы) |
|-----|--------|-----------|
| 1 — мост ассетов | ✅ | см. git log |
| 2 — shading modes | ✅ / 📌 | Unlit/Normals/Roughness/Depth → unavailable; Wireframe = mesh edges |
| 3 — небо, IBL, toggles | ✅ / 📌 | `set_sky_mode(Procedural)`; GI без SDF seed |
| 4 — snap + орто | ✅ | quad ortho уже на `QUAD_ORTHO_HALF`; snap API |
| 5 — screenshot + stats | ✅ | PNG readback; GPU `n/a` без profiler |
| 6 — UI / команды | ✅ | палитра + View toolbar |

## 📌 шаги и отсутствующие API

| Что | Почему 📌 |
|-----|-----------|
| Unlit / Normals / Roughness / Depth | В `pbr.wgsl` нет world-normal / roughness / depth / unlit `ShaderDebugVizMode`; не используется суррогат `ShadowSources` |
| Полный glTF PBR (MR/normal maps) | Нет `Material::from_gltf` / cook path в editor; только base colour + embedded albedo RGBA8 |
| GI «включён» без эффекта | Редактор не вызывает `set_gi_static_geometry` — нет SDF в editor world |
| SDF debug | Нет отдельного SDF viz; substitute `GiDebug::Visibility` |
| Procedural IBL в первом кадре | `ibl_resource.is_dummy()` до `prepare_procedural_sky` (runtime overlay) |
| GPU golden / wireframe diff | Тесты `#[ignore]`; координатор |

## Assumptions (merge)

- **`ViewportSceneAssetRef`** зеркалит `redox_scene::AssetRef` для `MeshHandle`; EDT-A8 может заменить тип без смены полей.
- Прокси-куб только если **нет** пути/ref или загрузка glTF провалилась (`ViewportMeshCache::missing_entities`).
- `EditorSettings.viewport` сохраняет toggles (`editor_settings.ron`).

## Файлы (основные)

- `crates/redox_editor_viewport/src/asset_bridge.rs`
- `crates/redox_editor_viewport/src/shading.rs`
- `crates/redox_editor_viewport/src/viewport_runtime.rs`
- `crates/redox_editor_viewport/src/viewport_screenshot.rs`
- `crates/redox_editor_viewport/src/snap.rs`
- `crates/redox_editor/src/render_host.rs`
- `crates/redox_editor/src/render_entities.rs`
- `crates/redox_editor_core/src/viewport_prefs.rs`
- `crates/redox_editor_core/src/settings.rs`
- `docs/engineering/KNOWN_LIMITS.md` (EDT-B3)
- Тесты: `crates/redox_editor_viewport/tests/b3_viewport.rs`, `settings_roundtrip.rs`, `gizmo_snap_grid.rs`

## Координатору проверить

```bash
cargo check -p redox_editor_viewport -p redox_editor -p redox_editor_core
cargo test -p redox_editor_viewport
cargo test -p redox_editor_viewport --test b3_viewport -- --ignored
clippy -D warnings -p redox_editor_viewport -p redox_editor
```

Ожидание: proxy path только без ассета; с glTF — real mesh draw; shading unavailable в overlay; F1–F4 без регрессии; screenshot PNG в temp.

## Выбор (одна строка)

Wireframe на реальном пути = Lit PBR + line overlay по рёбрам загруженного меша (не fresnel на кубах).