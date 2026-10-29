# MAT-02 — Материалы, E2: граф материалов → IR → WGSL (пошагово, 6 шагов)

- **Статус:** выполнены шаги 1–3 (крейт `redox_matgraph`); шаги 4–6 (GPU-сверка, кэш модулей, горячая перезагрузка) не сделаны.

- **Размер:** 6 шагов (S, M, M, M, M, S) ≈ 12–14 дней; **каждый шаг — отдельный слияемый коммит**. **Крейты:** новый `redox_matgraph` (чистый Rust, без `wgpu`) и `redox_render` (шаги 5–6). **Зависит от:** MAT-01 (`material_surface`, `SurfaceData`). **Блокирует:** MAT-03…MAT-07.
- **Модель:** haiku по шагам 1–2 и 6; шаги 3–5 — haiku **с проверкой координатора после каждого** (кодогенерация и варианты — самые хрупкие места).
- **Решение по структуре:** отдельный крейт `redox_matgraph` (граф, проверка, IR, генерация текста WGSL) — всё тестируется без GPU; в `redox_render` остаётся только создание шейдерных модулей. Зависимости: `serde`, `ron` (workspace), `thiserror`; `naga = "30"` как dev-зависимость для проверки сгенерированного WGSL (в `redox_render` уже есть).

## Суть
Граф материала — это редактор узлов, из которого получается шейдер одного материала. Ассет `.matgraph` (RON) → проверка типов → IR → WGSL-функция `material_surface` → вариант шейдера под проход → кэш → горячая перезагрузка.
Параметры, помеченные «параметр», идут в uniform экземпляра материала, чтобы сотни экземпляров одного графа делили один шейдер.

## Что уже есть
- `ron` и `serde` в workspace (`Cargo.toml:73–74`); образец RON-ассета с проверкой — `redox_anim/src/graph/` (граф анимации, `graph/tests.rs`) — смотреть стиль типов/ошибок.
- `naga 30` (`redox_render/Cargo.toml:61`), `create_shader_module`/`resolve_source` (`shader/mod.rs:273`), горячая перезагрузка `ShaderWatcher` (`shader/watch.rs`).
- После MAT-01: `SurfaceInput`, `SurfaceData`, `material_surface`.

## Шаг 1 — формат графа и проверка (S, чистая логика)
`redox_matgraph/src/graph.rs`:
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ty { F32, Vec2, Vec3, Vec4, Texture2d, Sampler }

pub type NodeId = u32;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PortRef { pub node: NodeId, pub port: u8 }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum NodeKind {
    // Входы
    Uv, WorldPos, Normal, ViewDir, VertexColor, Time,
    // Константы и параметры
    ConstF32(f32), ConstVec3([f32; 3]), Param { name: String, ty: Ty, default: [f32; 4] },
    // Математика (набор расширяется в MAT-03)
    Add, Sub, Mul, Lerp, Clamp, Dot, Normalize, Pow, OneMinus,
    // Выход
    SurfaceOutput,      // порты: base_color (Vec3), opacity (F32), normal (Vec3), metallic, roughness (F32), emissive (Vec3)
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node { pub id: NodeId, pub kind: NodeKind, pub inputs: Vec<Option<PortRef>> }   // позиционно по портам; None = порт по умолчанию

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaterialGraph { pub version: u32, pub name: String, pub nodes: Vec<Node> }

impl MaterialGraph {
    pub fn from_ron(text: &str) -> Result<Self, GraphError>;
    pub fn to_ron(&self) -> String;                 // детерминированный порядок (узлы по id)
    /// Checks ids, links, types, cycles and the output. Errors carry the node id.
    pub fn validate(&self) -> Vec<GraphError>;
}
```
Таблица типов портов: `fn input_types(kind) -> &'static [Ty]`, `fn output_type(kind, input_types) -> Ty` (для арифметики — тип по широчайшему входу с правилом «скаляр растягивается на вектор»; `Dot` → `F32`; `Normalize` → тип входа; `Lerp(a,b,t)` — `t` скаляр или того же типа).
Правила проверки (`GraphError { node: Option<NodeId>, message }`): дубликат id; связь на несуществующий узел/порт; несовместимый тип (с текстом «ожидалось …, получено …»); цикл (DFS; сообщение содержит цепочку id); нет `SurfaceOutput` или больше одного; неподключённый обязательный вход без значения по умолчанию; `Param` с пустым или повторяющимся именем; `version` ≠ 1.
Тесты (юнит, по одному на правило):
```rust
#[test] fn a_valid_graph_has_no_errors() { /* uv -> lerp(base, tint) -> output */ }
#[test] fn ron_roundtrip_is_lossless_and_deterministic() { /* from_ron(to_ron(g)) == g; to_ron одинаков для переставленных узлов */ }
#[test] fn a_cycle_is_reported_with_its_nodes() { /* a -> b -> a */ }
#[test] fn a_type_mismatch_names_the_node_and_both_types() { /* Vec3 в порт F32 */ }
#[test] fn a_missing_output_is_an_error() { /* ... */ }
#[test] fn two_outputs_are_an_error() { /* ... */ }
#[test] fn a_link_to_a_missing_node_is_an_error() { /* ... */ }
#[test] fn duplicate_param_names_are_an_error() { /* ... */ }
#[test] fn scalar_broadcast_is_allowed_in_arithmetic() { /* Mul(Vec3, F32) -> Vec3, ошибок нет */ }
#[test] fn an_unknown_version_is_rejected() { /* version 99 */ }
```

## Шаг 2 — IR и оптимизации (M, чистая логика)
```rust
pub struct Ir { pub ops: Vec<Op>, pub params: Vec<ParamSlot>, pub outputs: SurfaceOutputs }
pub struct Op { pub id: u32, pub kind: OpKind, pub args: Vec<u32>, pub ty: Ty }     // SSA: аргумент = id предыдущей операции
pub struct ParamSlot { pub name: String, pub ty: Ty, pub offset: u32 /* in f32 within the instance uniform */, pub default: [f32; 4] }

pub fn lower(graph: &MaterialGraph) -> Result<Ir, Vec<GraphError>>;        // validate + топологическая сортировка (детерминированно: по id)
pub fn optimize(ir: Ir) -> Ir;                                              // свёртка констант, удаление мёртвых операций, общие подвыражения
```
Раскладка параметров: `Param` занимают слоты по 16 байт (vec4-выравнивание: F32/Vec2/Vec3/Vec4 — один vec4-слот, чтобы раскладка WGSL = раскладка Rust), порядок — по имени (стабильно).
Тесты:
```rust
#[test] fn lowering_orders_operations_so_arguments_come_first() { /* для каждого op все args < op.id */ }
#[test] fn constants_are_folded() { /* Add(Const 1, Const 2) -> Const 3; число ops уменьшилось */ }
#[test] fn dead_branches_are_removed() { /* узел без пути к выходу пропадает */ }
#[test] fn common_subexpressions_are_shared() { /* два одинаковых Mul(uv, 2) -> один op */ }
#[test] fn parameters_get_stable_16_byte_slots_sorted_by_name() { /* offset 0, 4, 8 ... */ }
#[test] fn optimisation_does_not_change_the_result_of_a_constant_graph() { /* вычислитель IR в тесте (интерпретатор на f32) даёт то же значение до и после */ }
```
Интерпретатор IR для тестов (`fn eval(&Ir, inputs: &EvalInputs) -> SurfaceValues`, `#[cfg(test)]` или `pub(crate)`) нужен как «эталон» для шага 4.

## Шаг 3 — кодогенерация WGSL (M)
```rust
pub struct CodegenOptions { pub pass: Pass /* Main | Depth | Shadow */, pub features: Features /* alpha_test, skinning, instanced, vertex_anim */ }
pub struct Generated { pub wgsl: String, pub uniform_wgsl: String, pub param_bytes: usize }
pub fn generate(ir: &Ir, opts: &CodegenOptions) -> Generated;
```
Генерируется: `struct MaterialParams { ... }` (vec4-слоты по `ParamSlot`) и функция `fn material_surface_graph(in: SurfaceInput, params: MaterialParams) -> SurfaceData`; каждая `Op` → `let v{id}: type = ...;` в порядке IR (SSA, без ветвлений в v1).
Для прохода `Depth/Shadow`: генерируются только операции, влияющие на `opacity` (мёртвые для остальных удаляются повторным прогоном DCE по выбранным выходам) и отсутствует освещение.
Тесты:
```rust
#[test] fn the_generated_code_passes_naga_validation() { /* обёртка: заголовок SurfaceInput/SurfaceData (минимальная копия из material_surface.wgsl) + generated; naga::front::wgsl::parse_str + naga::valid::Validator::validate == Ok */ }
#[test] fn generation_is_deterministic() { /* дважды -> строки равны */ }
#[test] fn a_depth_variant_contains_no_unused_texture_samples() { /* граф с текстурой в base_color и константой opacity: wgsl глубины не содержит 'textureSample' */ }
#[test] fn parameters_appear_in_the_uniform_struct_in_slot_order() { /* ... */ }
#[test] fn every_node_kind_has_a_generator() { /* перебор NodeKind: generate не паникует и проходит naga для минимального графа с этим узлом */ }
```
Файл `codegen.rs` ≤ 500 строк; таблица «узел → выражение WGSL» — отдельным `match` в `emit.rs`.

## Шаг 4 — сверка WGSL с интерпретатором на GPU (M, нужен `redox_render`)
GPU-тест `redox_render/tests/matgraph_gpu.rs` (пропуск без адаптера): compute-шейдер запускает сгенерированную функцию на одном входе (UV = (0.25, 0.75), …), записывает `SurfaceData` в буфер, тест читает и сравнивает с интерпретатором IR (допуск 1e-4). Графы: константа; `lerp` двух цветов по `uv.x`; параметры (значения из uniform); `Pow/Normalize/Dot`.
Это единственное место, где доказывается, что **кодогенерация считает то же, что IR** (без него «оба неверны одинаково» невозможно поймать).

## Шаг 5 — варианты, кэш, подключение (M)
- Ключ варианта: `(хеш графа, Pass, Features)` → `ShaderKey(u64)`; хеш — стабильный (FNV-1a или `std::hash` с фиксированным ключом — **не** `DefaultHasher` со случайным состоянием; см. `cache_key` в `virtual_geometry/cache.rs` — там `DefaultHasher::new()` детерминирован, допустимо).
- `MaterialShaderCache { modules: HashMap<ShaderKey, wgpu::ShaderModule>, ... }` в `redox_render`: собирает итоговый текст = `material_surface.wgsl`-модуль (MAT-01) + `generated.wgsl` + остальной `pbr.wgsl`, создаёт модуль, **валидирует naga до** `create_shader_module` (ошибка → материал остаётся на предыдущем/запасном шейдере, в лог — сообщение с `node id`).
- Дисковый кэш текста (не бинарей): `.cache/matgraph/<key>.wgsl` — только для отладки/ускорения повторной генерации (необязательно; бинарный кэш конвейеров уже есть — `pipeline_cache.rs`).
- Подключение: `MaterialInstance { graph: Some(id), .. }` (поле заведено в MAT-01 шаг 3) → отрисовка использует конвейер варианта; материалы без графа идут прежним конвейером (**побитно** прежняя картинка).
- Uniform экземпляра: буфер параметров на экземпляр, биндится в группе материала (group 2 — сейчас `legacy_material_bind_group`; **ПРОВЕРИТЬ** `registry.rs`, как расширить без смены остальных групп).
- Тесты (GPU): `a_graph_material_renders_the_expected_colour` (граф «константа красный» → пиксель куба красный с учётом освещения; сравнение с `Material::solid(red)` в допуске 2/255); `two_instances_share_one_shader_with_different_params` (число созданных модулей = 1 на 2 экземпляра, цвета разные); `a_broken_graph_keeps_the_previous_shader` (подмена на невалидный: кадр рендерится, лог содержит ошибку); `variants_are_cached` (повторный запрос того же ключа не создаёт новый модуль).

## Шаг 6 — горячая перезагрузка (S)
`MatGraphWatcher` (по образцу `ScriptWatcher` из `redox_script/src/watch.rs`): опрос mtime/размера `.matgraph`, `poll() -> Vec<GraphReload>`; при изменении — `from_ron` → validate → пересборка вариантов графа → замена модуля; ошибка показывается один раз, старый шейдер продолжает работать.
Тесты: `a_changed_file_triggers_one_reload`; `a_broken_file_reports_once_and_keeps_the_old_variant`; `an_unchanged_file_does_nothing`. Пример-ассет `assets/materials/example_tint.matgraph` (только если есть каталог ассетов примеров; иначе в `tests/data/`).
Абзац в `PLAN_MATERIALS.md` §1.2, `PLAN_EXECUTION.md` E2, `CHANGELOG.md`, `docs/CRATES.md` (генератором).

## Границы
- **Читать:** MAT-01, `PLAN_MATERIALS.md` §1–1.3, `redox_anim/src/graph/*`, `shader/{mod,watch}.rs`, `pass/pbr.rs`, `registry.rs`, `redox_script/src/watch.rs`.
- **Создать:** `crates/redox_matgraph/**`, `redox_render/src/matgraph_cache.rs`, `redox_render/tests/matgraph_gpu.rs`.
- **Менять:** корневой `Cargo.toml` (members), `redox_render/Cargo.toml` (+ `redox_matgraph`), `registry.rs`, `renderer/*` (по шагу 5), `docs/CRATES.md`.
- **Не трогать:** `pbr.wgsl` сверх MAT-01, тени, VG, эталоны.

## Проверка
`cargo fmt`, `cargo clippy -p redox_matgraph -p redox_render --all-targets -- -D warnings`, `cargo test -p redox_matgraph`, `cargo test -p redox_render --test matgraph_gpu --test golden_frames`, `cargo deny check`, `python scripts/{gen_crates_doc.py --check,check_file_sizes.py}`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. MAT-01 обязан быть в `main`. Редактор узлов — НЕ здесь (этап I); граф правится текстом.
