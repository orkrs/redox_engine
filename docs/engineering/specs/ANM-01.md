# ANM-01 — Тангенты в `SkinnedGeometry`: данные, импорт glTF, деформация на CPU

- **Статус:** выполнено 2026-10-29, принято в `main`. Знак ручности (w тангента) не хранится: GPU считает его правилом CPU (`n × t · Y`).

- **Размер:** S. **Крейт:** `redox_anim`. **Зависит от:** ничего. **Блокирует:** RND-01, RND-02.
- **Модель:** haiku.

## Суть
`SkinnedGeometry` (`crates/redox_anim/src/skinning.rs:7`) хранит позиции, нормали, UV, кости и веса, но не тангенты.
Рендер подставляет `DEFAULT_TANGENT = [1,0,0]` (`redox_render/src/skinning.rs:13`), поэтому карты нормалей скиннинговых
персонажей освещаются неверно. Здесь — только данные и CPU-деформация; рендер подключают RND-01 (CPU) и RND-02 (GPU).

## Границы
- **Читать:** `skinning.rs` (структура, `validate`, `deform_into`), `gltf_import.rs` (`build_geometry`, строка ~100), `procedural.rs` (генераторы геометрии), `benches/skinning.rs`.
- **Менять:** `skinning.rs`, `gltf_import.rs`, `procedural.rs` (если там геометрия собирается литералом), `benches/skinning.rs` (литерал), тесты `crates/redox_anim/tests/` или модуль `tests` в файле.
- **Не трогать:** `redox_render`, анимационный граф, клипы.

## Решение
1. Поле: `pub tangents: Vec<[f32; 3]>` в `SkinnedGeometry` (после `normals`). **Пустой вектор допустим и означает «тангентов нет»**
   (так старые данные и генераторы остаются валидными). `validate`: `tangents.is_empty() || tangents.len() == vertex_count`, иначе `Err`.
2. Метод `pub fn has_tangents(&self) -> bool { !self.tangents.is_empty() }`.
3. `pub fn deform_tangents_into(&self, skin: &[Mat4], out: &mut Vec<[f32; 3]>)`: для каждой вершины тот же смешанный матричный блок, что в
   `deform_into` (нормализованные веса; влияние вне `skin.len()` пропускается; сумма весов ≈ 0 → единичная матрица), результат
   `normalize((M * vec4(t, 0)).xyz)`, нулевой вектор остаётся нулевым. Если `tangents` пусто — `out.clear()`. Никаких выделений при повторном вызове с тем же `out`.
   Общий кусок «смешанная матрица вершины» вынести в приватную функцию и использовать в обоих методах (не копировать).
4. Импорт glTF (`build_geometry`): `reader.read_tangents()` даёт `[f32; 4]` → брать `[x, y, z]`. Если у **всех** примитивов есть `TANGENT` —
   заполнять; если только у части — для остальных вставить `[1.0, 0.0, 0.0]` (иначе длины разойдутся); если ни у одного — оставить пусто.
   Проверку длин (`any(|&n| n != count)`) дополнить длиной тангентов, когда они есть.

## Пошагово
1. Написать тесты (ниже), убедиться что красные (не компилируются — допустимо; для этого шага добавьте поле и `todo!()`).
2. Поле, `validate`, `has_tangents`, `deform_tangents_into` с общей приватной функцией.
3. Импорт glTF, правка литералов `SkinnedGeometry { .. }` (искать `SkinnedGeometry {` и `SkinnedGeometry::default()` в `crates/`).

## Тесты
```rust
#[test]
fn validate_rejects_a_tangent_array_of_the_wrong_length() {
    let mut g = quad_geometry();              // helper: 4 vertices, one bone, tangents empty
    assert!(g.validate(1).is_ok());           // empty tangents are fine
    g.tangents = vec![[1.0, 0.0, 0.0]; 3];
    assert!(g.validate(1).is_err());
    g.tangents = vec![[1.0, 0.0, 0.0]; 4];
    assert!(g.validate(1).is_ok());
}
#[test]
fn tangents_rotate_with_the_bone_and_stay_unit_length() {
    let mut g = quad_geometry();
    g.tangents = vec![[1.0, 0.0, 0.0]; 4];
    let skin = [Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2)];
    let mut out = Vec::new();
    g.deform_tangents_into(&skin, &mut out);
    for t in &out { assert!((Vec3::from(*t) - Vec3::Y).length() < 1e-5); }
}
#[test]
fn a_geometry_without_tangents_produces_none() {
    let g = quad_geometry();
    let mut out = vec![[9.0; 3]];
    g.deform_tangents_into(&[Mat4::IDENTITY], &mut out);
    assert!(out.is_empty());
}
#[test]
fn a_bone_index_out_of_range_is_ignored_like_in_deform_into() {
    // vertex with joints [5,0,0,0] weights [0.5,0.5,0,0], skin.len()==1 -> uses bone 0 only (normalised); no panic
}
#[test]
fn the_gltf_import_reads_tangents() {
    // The test glTF used by the existing import tests (see gltf_import tests / fixtures) has no TANGENT.
    // Build a minimal glTF in memory (embedded base64 buffer) with one skinned triangle and a TANGENT accessor,
    // import, assert geometry.has_tangents() and tangents[0] == [1.0, 0.0, 0.0].
}
```
`quad_geometry()` — маленький помощник внутри тестов: 4 вершины в плоскости XY, `joints = [[0,0,0,0];4]`, `weights = [[1.0,0.0,0.0,0.0];4]`, нормали `[0,0,1]`.
Для последнего теста взять готовый встроенный glTF из существующих тестов импорта (`grep -n "data:application" crates/redox_anim`), если нет — написать
простейший `.gltf` с `data:`-буфером руками и объяснить в комментарии теста раскладку байтов.

## Проверка
`cargo test -p redox_anim`, `cargo bench -p redox_anim --no-run` (компиляция бенча), `cargo clippy -p redox_anim --all-targets -- -D warnings`; сборка всего workspace `cargo check --workspace --all-targets` (литералы в других крейтах).

## Для исполнителя
`git merge --ff-only main`. Не меняй `redox_render` — если литерал `SkinnedGeometry` есть там, добавь `tangents: Vec::new()` минимально и скажи об этом.
