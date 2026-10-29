# MAT-06 — Материалы, E6: режимы прозрачности — masked, dithered, translucent, additive, преломление

- **Статус:** выполнен шаг 2 (additive) и тесты стекла; шаги 3–5 не сделаны.

- **Размер:** 5 шагов (S, S, M, M, S) ≈ 8 дней. **Крейт:** `redox_render`. **Зависит от:** MAT-01. **Блокирует:** MAT-07.
- **Модель:** haiku.
- Объём — «минимум» плана E6. **Не входит:** OIT (weighted blended) — только если появятся частицы со многими слоями (в плане «полное»).

## Суть и аудит
Аудит кода показал, что часть режимов уже есть:
| Режим | Состояние | Где |
|---|---|---|
| Opaque | есть | основной проход |
| Masked (alpha test) | **есть в основном проходе**, флаг `material.flags & 16u` | `pbr.wgsl:~696–707` (стохастический порог), `MaterialUniform.flags` |
| Dithered | **есть** (тот же код по сине-шумовой текстуре + золотое сечение по кадрам) | `pbr.wgsl:~700–706` |
| Translucent (alpha blend) | **есть**, `BlendMode::Alpha`: отдельный проход, сортировка от дальних к ближним по центру `bounds`, глубина не пишется | `asset_types.rs:43–50`, `pass/transparent.rs`, `renderer/draw.rs:107+` |
| Additive | **нет** (`BlendMode` = `Opaque \| Alpha`) | — |
| Refraction | **нет** | — |
| Masked в теневых проходах | **нет** (тень не вырезается по альфе) — проверить | `shaders/shadow.wgsl`, `vsm_depth.wgsl` |
| Реактивная маска TAA для прозрачного | **ПРОВЕРИТЬ** (`t_reactive` в `taa.wgsl:15`) | — |
Остальное в этой спеке — добавить недостающее и доказать тестами, что существующее работает.

## Шаг 1 — тесты существующего поведения (S)
Прежде чем менять, зафиксировать: `tests/transparency.rs` (GPU, пропуск без адаптера; сцена — куб за стеклом):
```rust
#[test] fn an_opaque_cube_behind_alpha_glass_is_visible_through_it() { /* пиксели в области стекла = mix(фон куба, цвет стекла, alpha) ± 0.05 */ }
#[test] fn glass_does_not_write_depth() { /* второй стеклянный куб за первым виден сквозь первый */ }
#[test] fn sorting_is_back_to_front() { /* красное стекло ближе синего, затем поменять местами: цвета в области перекрытия отличаются как ожидается */ }
#[test] fn an_alpha_tested_leaf_has_holes() { /* текстура/альфа 0 в половине: в кадре есть пиксели фона внутри AABB листа */ }
#[test] fn dithered_fade_changes_coverage_gradually() { /* color.a 1.0, 0.5, 0.1: доля покрытых пикселей убывает монотонно */ }
```
Если какой-то тест красный на текущем коде — **это находка**: остановиться, записать, не править (решает координатор).

## Шаг 2 — Additive (S)
- `BlendMode::Additive` в `asset_types.rs` (+ парсер материалов, сериализация: **ПРОВЕРИТЬ** где разбирается `BlendMode`: `grep -rn "BlendMode" crates`), конвейер `pass/transparent.rs`: вариант с `BlendState { color: src One, dst One, add; alpha: one one }`, без сортировки (порядок не важен), `has_alpha_objects` учитывает оба режима, список аддитивных объектов — отдельный проход после стекла (чтобы огонь не перекрывался стеклом некорректно).
- Тесты: `additive_surfaces_brighten_the_background` (пиксель = фон + цвет·alpha ± 0.02); `additive_order_does_not_matter` (два перекрывающихся — разные порядки в списке дают одинаковый кадр побитно); `opaque_frames_are_unchanged` (golden).

## Шаг 3 — Masked и прозрачность в тенях (M)
- Теневые проходы (`shaders/shadow.wgsl`, `vsm_depth.wgsl`, VG-тень — позже MAT-07) для материалов `flags & 16u` (alpha test): читать альфу той же функцией `material_surface`-упрощённой («только opacity», вариант `Pass::Depth/Shadow` из MAT-02 — но без графа: общая WGSL-функция `material_opacity(uv, vertex_color, material)`) и `discard`-ить по порогу (без dither: в тени шум нужен меньше; порог 0.5).
- Требуется доступ к материалу и текстуре из теневого прохода: **ПРОВЕРИТЬ** привязки теневых пайплайнов (`pass/shadow.rs`, `shadow/depth_pass.rs`) — сейчас у них нет группы материала. Добавить группу материала только для варианта alpha-test (отдельный конвейер `shadow_masked_pipeline`, переключение по `blend_flags`/`alpha_test_flags` на объект — `Renderer::material_alpha_test_flags: Vec<bool>` по образцу `material_blend_flags`).
- Тесты (GPU): `a_masked_leaf_casts_a_shadow_with_holes` (тень от листа с дырой на плоскости: в области тени есть освещённые пиксели); `an_opaque_caster_is_unchanged` (golden); `masked_shadows_work_in_vsm_pages` (VSM включён).

## Шаг 4 — Преломление (M)
- `BlendMode::Refractive { ior: f32, tint: [f32;3] }` или отдельное поле `Material::refraction: Option<Refraction { ior, strength, roughness_blur }>` (**выбор:** отдельное поле — режим смешивания остаётся `Alpha`; решение записать).
- Копия цвета сцены после непрозрачного прохода (и до стекла): `scene_color_copy` с цепочкой мипов для шероховатости (`copy_texture_to_texture` HDR → мип 0, затем даунсэмпл — переиспользовать `bloom` даунсэмплер, **ПРОВЕРИТЬ** `post/bloom.rs`); пересоздание в `resize.rs`; создаётся только если в кадре есть преломляющие материалы.
- Шейдер: в `fs_main` ветка `refraction`: `offset_uv = view_normal.xy * (ior - 1) * strength / max(dist, 1)`; цвет `textureSampleLevel(scene_copy, uv + offset, roughness*mip_count)`, смешивается с освещением поверхности по Френелю; защита от выборки из пикселей перед стеклом (если глубина в точке смещения < глубины стекла → смещение 0).
- Чистая математика в `post/refraction.rs` + тесты: `refraction_offset_is_zero_at_ior_one`, `offset_grows_with_the_normal_tilt`, `offset_shrinks_with_distance`, `mip_level_scales_with_roughness_and_is_clamped`.
- Тесты (GPU): `glass_with_ior_displaces_the_background` (полоса на фоне искажается в области стекла: пиксели сдвинуты ≥ 2 px); `ior_one_matches_plain_alpha_glass` (±0.02); `no_scene_copy_without_refractive_materials` (флаг-счётчик).

## Шаг 5 — взаимодействие с TAA и документация (S)
- Прозрачные поверхности пишут реактивную маску (`t_reactive`, `taa.wgsl:265–270`): **ПРОВЕРИТЬ** чем заполняется маска сейчас; добавить запись `reactive = alpha` для прозрачного прохода (отдельный вывод — только если заполнение уже вынесено в общий вывод; иначе — вне объёма, записать в отчёт).
- Тест: `moving_glass_leaves_no_ghost_trail` (метрики из PST-04: `bright_pixels` позади движущегося стекла ≤ допуск).
- Абзац в `PLAN_MATERIALS.md` §4, `PLAN_EXECUTION.md` E6, `CHANGELOG.md`.

## Границы
- **Читать:** `asset_types.rs`, `material.rs`, `pass/{transparent,shadow}.rs`, `shadow/depth_pass.rs`, `shaders/{pbr,shadow,vsm_depth,taa}.wgsl`, `renderer/draw.rs`, `renderer/frame/post.rs`, `post/bloom.rs`, PST-04 (метрики).
- **Создать:** `tests/transparency.rs`, `post/refraction.rs`.
- **Менять:** `asset_types.rs`, `material.rs`, `pass/{transparent,shadow}.rs`, `shaders/{pbr,shadow,vsm_depth}.wgsl`, `renderer/*`.
- **Не трогать:** SSS, SSR.

## Проверка
`cargo fmt`, `cargo clippy -p redox_render --all-targets -- -D warnings`, `cargo test -p redox_render --test transparency --test golden_frames --test instancing`, `cargo check --workspace --all-targets` (изменён публичный enum `BlendMode`; `cargo test --workspace` запускает координатор), `python scripts/check_file_sizes.py`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Шаг 1 — только тесты, ничего не меняй в коде; красные тесты шага 1 — в отчёт, шаги 2+ не начинай до решения координатора.
