# ANM-02 — Морфы (blend shapes): импорт glTF, веса в клипе, деформация на CPU

- **Статус:** выполнено 2026-10-29; веса подключены к `SkinnedMeshBinding::set_morph_weights`.

- **Размер:** M (три шага по S). **Крейт:** `redox_anim`; шаг 4 — `redox_render` (см. ANM-03). **Зависит от:** ANM-01 (тангенты, готово). **Блокирует:** ANM-03.
- **Модель:** haiku.

## Суть
Лицевая анимация и корректирующие формы (blend shapes / morph targets) в движке не поддерживаются: `gltf_import.rs:209` пропускает канал `MorphTargetWeights` (`=> {}`),
в `SkinnedGeometry` нет ни целей, ни весов. Пункт плана «Морфы» был отложен словами «нечем проверять без ассета с морфами». **Это не препятствие**: тесты движка собирают `.glb` в памяти
(`gltf_import.rs:281–330`, `sample_glb_with(tangents)`: байты буфера + JSON-шаблон), так же собирается и ассет с морфами; живой ассет нужен только для финальной проверки глазами.

## Что уже есть (проверено чтением)
- `SkinnedGeometry` (`skinning.rs:7`): `positions, normals, tangents, uvs, joints, weights, indices`; `deform_into(skin, out_pos, out_nor)` (86), `deform_tangents_into` (70), `validate` (31).
- `AnimationClip` (`skeleton.rs:206`): `name, duration, tracks: HashMap<String, Vec<Keyframe>>`; `sample(skeleton, time, looping)` (223).
- Импорт: `load_glb` → `build_model` → `build_clips` (`gltf_import.rs:14,…`); примитивы одного меша склеиваются в одну геометрию со смещением индексов `base`.
- Рендер: деформированные данные грузятся в меш через `Renderer` (`renderer/assets.rs:29` — «Overwrites the vertices of an uploaded mesh in place (skinning, morphing)»), GPU-скиннинг — `skinning_gpu.rs`, `shaders/skin_compute.wgsl`.
- **ПРОВЕРИТЬ:** сколько мест строят `SkinnedGeometry { … }` литералом без `..Default::default()`: `grep -rn "SkinnedGeometry {" crates examples`. Новое поле сломает их; добавить `..Default::default()` или `morph_targets: Vec::new()`.

## Шаг 1 — данные и деформация (S, без импорта)
```rust
/// One morph target: per-vertex offsets from the bind pose (glTF `targets[i]`). Arrays are either empty (not animated) or as long as the geometry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MorphTarget {
    pub name: String,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 3]>,
}
// SkinnedGeometry gets:
pub morph_targets: Vec<MorphTarget>,
pub morph_defaults: Vec<f32>,   // glTF mesh.weights; empty = all zero

impl SkinnedGeometry {
    /// Adds the weighted offsets of all morph targets to the bind pose. Weights shorter than the target list count as 0, longer are ignored,
    /// NaN counts as 0. Targets with an empty array contribute nothing to it. Output vectors are cleared and refilled.
    pub fn morph_into(&self, weights: &[f32], out_positions: &mut Vec<[f32; 3]>, out_normals: &mut Vec<[f32; 3]>);
    /// Skinning on top of morphing: `deform_into`, but the bind pose is first morphed with `weights`.
    pub fn deform_morphed_into(&self, skin: &[Mat4], weights: &[f32], out_positions: &mut Vec<[f32; 3]>, out_normals: &mut Vec<[f32; 3]>);
}
```
Правила: нормали после морфа нормализуются (`normalize_or_zero`); `validate` проверяет длины массивов целей (пустые допустимы); без целей `deform_morphed_into` == `deform_into` (побитно).
Реализация `deform_morphed_into`: морфить в временный буфер позиций/нормалей, затем блендить матрицы так же, как `deform_into` (вынести общий внутренний цикл, не копировать `blend_matrix`).
Тесты (юнит, `skinning.rs`):
```rust
#[test] fn zero_weights_leave_the_bind_pose() { /* one target, weights [0.0] -> morph_into == positions */ }
#[test] fn a_full_weight_adds_the_offset() { /* target +1 on Y for vertex 0; weights [1.0] -> y + 1 */ }
#[test] fn weights_add_up_linearly() { /* two targets, weights [0.5, 0.25] -> sum of halves */ }
#[test] fn missing_and_nan_weights_count_as_zero() { /* weights [] and [NAN] */ }
#[test] fn an_empty_normal_array_keeps_the_bind_normals() { /* target with positions only */ }
#[test] fn without_targets_the_morphed_deform_equals_the_plain_one() { /* assert_eq! of the outputs */ }
#[test] fn morphing_happens_before_skinning() { /* offset along X, bone rotated 90° about Z: result offset follows the rotation */ }
#[test] fn validate_rejects_a_target_of_the_wrong_length() { /* positions.len() != n -> Err */ }
```

## Шаг 2 — импорт целей и весов (S)
В `build_model`/`geometry`-сборке (`gltf_import.rs`): для каждого примитива `reader.read_morph_targets()` → `(Option<positions>, Option<normals>, Option<tangents>)` итераторы с дельтами;
цели нумеруются по порядку в glTF (`targets[i]`), у всех примитивов меша их число одинаково (**ПРОВЕРИТЬ** спецификацию glTF §3.7.2.2: да, обязано совпадать; при расхождении — `Err("primitives have different morph target counts")`).
Склейка примитивов: если у примитива нет данных цели — добавить нули нужной длины, чтобы массивы цели совпали с общей длиной геометрии.
Имена целей: `mesh.extras`/`targetNames` — **ПРОВЕРИТЬ**, отдаёт ли крейт `gltf` (`mesh.extras()`), иначе `morph_{i}`.
`morph_defaults` — из `mesh.weights()`.
Тест-ассет в памяти: расширить `sample_glb_with` (или новый `sample_glb_morph()`): треугольник из 3 вершин, одна цель со смещением `[0, 0.5, 0]` у вершины 0, и цель с нормалями; JSON получает `"targets": [{"POSITION": N}]` в примитиве и `"weights": [0.0]` в меше.
Тесты:
```rust
#[test] fn morph_targets_are_imported_with_their_offsets() { /* load_glb -> geometry.morph_targets.len() == 1; positions[0] == [0.0, 0.5, 0.0] */ }
#[test] fn default_weights_come_from_the_mesh() { /* weights [0.25] -> morph_defaults == [0.25] */ }
#[test] fn a_model_without_targets_has_none() { /* sample_glb() -> morph_targets.is_empty() */ }
#[test] fn mismatched_target_counts_between_primitives_are_rejected() { /* if the in-memory asset allows two primitives */ }
```

## Шаг 3 — веса в клипе (S)
```rust
// AnimationClip gets:
/// Keyframes of the morph weights (one value per target) sorted by time; empty = the clip does not animate morphs.
pub morph_weights: Vec<(f32, Vec<f32>)>,
impl AnimationClip {
    /// Linear interpolation between neighbouring keyframes; clamps outside; wraps with `looping`. `count` is the number of targets of the model: shorter keyframes are padded with 0.
    pub fn sample_morph_weights(&self, time: f32, looping: bool, count: usize) -> Vec<f32>;
}
```
Импорт (`build_clips`, строка 209): `ReadOutputs::MorphTargetWeights(w)` → плоский массив `times.len() * target_count`, разрезать на кадры по `target_count` (из `node.mesh().primitives().next().morph_targets().count()`; для кубического сплайна (`cubic`) — значения как в `pick`: триплеты, брать среднее).
Канал привязан к узлу с мешем; один меш на модель — достаточно (канал другого узла игнорируется, **документировать**).
Тесты: `a_clip_samples_morph_weights_between_keyframes` (0.0 → 1.0 за 1 с, в 0.5 с вес 0.5); `weights_clamp_outside_the_clip`; `a_looping_clip_wraps_morph_weights`; `short_keyframes_are_padded_with_zero`;
интеграционный `an_imported_clip_animates_the_morph_weights` (в памяти собранный glb с каналом `weights`).

## Шаг 4 — подключение к игре (S)
`Animator`/система скиннинга, вызывающая `deform_into`: **ПРОВЕРИТЬ** `grep -rn "deform_into" crates examples` — заменить вызов на `deform_morphed_into(skin, &weights, …)`, где `weights = clip.sample_morph_weights(...)`, если у модели есть цели; иначе путь прежний (побитно).
Для GPU-пути (`skinning_gpu.rs`) морфы пока считаются на CPU и грузятся как «позиции после морфа до скиннинга»? — **нет**: GPU-путь морфы игнорирует до ANM-03; в doc-комментарии написать явно.

## Границы
- **Читать:** `skinning.rs`, `gltf_import.rs` (целиком), `skeleton.rs:200–260`, `animator.rs`, `lib.rs`, места вызова `deform_into` (grep).
- **Менять:** `skinning.rs`, `gltf_import.rs`, `skeleton.rs`, `lib.rs` (реэкспорт `MorphTarget`).
- **Не трогать:** `redox_render` (до ANM-03), IK, граф анимации.

## Проверка
`cargo fmt`, `cargo clippy -p redox_anim --all-targets -- -D warnings`, `cargo test -p redox_anim`, `cargo check --workspace --all-targets` (из-за нового поля; `cargo test --workspace` запускает координатор), `python scripts/check_file_sizes.py` (`gltf_import.rs` растёт — тесты не выносить, но следить за 800 строками; если превысит — вынести тестовые ассеты в `gltf_import/test_assets.rs`).
Живая проверка владельцем: лицо с блендшейпами из Blender (экспорт glTF с shape keys) — после ANM-03.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Каждый шаг — отдельный коммит; прежние тесты зелёные без правок.
