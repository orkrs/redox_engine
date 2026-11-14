# EDT-B2 implementation report

- **Branch:** `edt/b2`
- **Worktree:** `/home/besyak/Документы/redox/.worktrees/b2`
- **Status:** ✅ (coordinator must run cargo)

## Commits (intended order)

1. Step 1 — `feat: component catalog and dependency prompts — EDT-B2 step 1`
2. Step 2 — `feat: component add/remove/copy transactions — EDT-B2 step 2`
3. Step 3 — `feat: inspector add/remove/copy UI — EDT-B2 step 3`
4. Step 4 — `feat: gameplay/scene reflect categories — EDT-B2 step 4`

## What was delivered

### Step 1 — `ComponentCatalog`

- New `crates/redox_editor_core/src/component_catalog.rs`: entries, search, addable list, tooltips, exclusion of editor-only / all-hidden-field types.
- Soft physics table: `Collider` → recommends `RigidBody` (warn + Continue/Cancel, not auto-add).
- `dependency_warning_for_add` + `resolve_dependency_prompt` (file_state-style pure functions).
- Tests: `crates/redox_editor_core/tests/component_catalog.rs`.

### Step 2 — undo transactions

- New `crates/redox_editor_core/src/component_commands.rs`: `component_add`, `component_remove`, `component_copy_values`, `record_component_transaction`, `bulk_add_component`.
- Tests: `crates/redox_editor_core/tests/component_commands.rs` (add+undo, remove+undo, copy×3 one step, bulk add skip + `undo.len() == 1`).

### Step 3 — UI

- `crates/redox_editor_ui/src/inspector.rs`: catalog search in Add menu, Remove per section, Copy from primary, dependency Continue/Cancel, single `record_component_transaction` per frame batch.

### Step 4 — registration / categories

- `AttributeSet`: `reflect_schema` with category **Gameplay**.
- `Name` / `Tag`: category **Scene** (+ tooltips).
- Already registered via `register_scene_components`: Transform, Name, Tag, lights, Camera, handles, AssetRefs, Velocity, AttributeSet.

## Registered categories (inspector add menu)

| Category   | Types (registered) |
|-----------|---------------------|
| Transform | Transform |
| Scene     | Name, Tag |
| Rendering | DirectionalLight, PointLight, SpotLight, Camera, MeshHandle, MaterialHandle, AssetRefs |
| Physics   | Velocity |
| Gameplay  | AttributeSet |

## Assumptions

- Soft `Collider`/`RigidBody` warning uses string names from the physics API even though rapier `Collider`/`RigidBody` are not reflect-registered yet.
- `requires` metadata still auto-adds dependencies; soft prerequisites only warn.
- Inspector dependency prompt is inline (Continue/Cancel buttons); no modal window in core.

## Gaps (📌)

- **Audio:** `AudioEmitter`, `AudioListener` — no `Reflect` / not scene-safe to register.
- **Physics:** `RigidBody`, `Collider`, `CharacterController`, `Kinematic`, `Trigger` — runtime handles, not in registry (see `KNOWN_LIMITS.md` in register).
- **Gameplay:** `Ability`, `ActiveEffects`, `StateTreeComponent` — no `Reflect`.
- **Catalog:** real `Collider` type not addable until a saved reflect stub exists (B7).

## Coordinator: verify

```bash
cd /home/besyak/Документы/redox/.worktrees/b2
cargo fmt
cargo clippy -p redox_editor_core -p redox_editor_ui -p redox_reflect -p redox_scene -p redox_gameplay -- -D warnings
cargo test -p redox_editor_core -p redox_editor_ui -p redox_reflect
```

Expect: new tests in `component_catalog.rs` and `component_commands.rs` pass; existing `inspector_model` tests unchanged in behaviour.

Manual smoke: multi-select → Add Component (search) → dependency prompt on Collider name → Remove → Undo/Redo.