# RedOx Engine — Uplift v2 Status

> Master plan: `.cursor/plans/redox_uplift_v2_414efd3e.plan.md` (Cursor plan file)

**Uplift v2 runtime phases 0–8:** закрыты по каркасу. Опциональные хвосты качества — ниже.

---

## ✅ Закрыто

### Phase 0–3
ECS, Renderer/FrameView, VSM fixes, VG visibility+resolve, GPU/CPU cull, VG shadows — см. предыдущие секции.

### Phase 4 — ScatterLight
- **`LightingMode::ScatterLight`** — RedOx stochastic K=8 point-light path
- **`scatter_light.wgsl`** + **`ScatterLightPass`** wired в `record_hdr_post_fx`
- PBR clustered points отключаются через `point_light_path` uniform
- Showcase / Prism Vault: toggle **ScatterLight path**

### Phase 5 — TSR + DDGI + SDF + Bloom
- **TSR:** `EngineConfig.render_scale` + slider (0.5–1.0); `Renderer::uses_tsr()`; TAA Catmull-Rom upscale при `render_scale < 1.0`
- **DDGI GPU:** 8×4×8 grid, `@binding(22–23)`, trilinear + octahedral lookup в PBR
  - ⚠️ **не** GPU RT: probes заполняются на CPU (directional + sky/ground)
- **SDF GPU:** 128³ atlas + **8-slot page table**, `@binding(26–28)`, multi-brick demo (ground/pillar/crate)
- **Bloom:** luminance threshold + **6-mip** downsample → tent upsample → composite (HDR `COPY_DST`)

### Phase 6 — Substrate
- **`MaterialLayer` / `MaterialData::layers`** + **`eval_substrate_lit` в `pbr.wgsl`** (directional / clustered / IBL)
  - VG resolve теперь тоже считает слои (base + 2 layers + sheen, directional / clustered / IBL); не полный Unreal Substrate graph

### Phase 7 — World Partition + PCG
- **`WorldPartition::open` / `in_memory`**, `tick` / `tick_streaming`
- **`StreamedActor` + `Transform`**, OFPA cells, streaming budget + hysteresis
- **PCG CLI:** `cargo run -p redox_scene --bin pcg -- --out worlds/arena --kind arena …`
- **Prism Vault** тикает partition от позиции игрока; стримит props/walls

### Phase 8 — Gameplay / VFX / Audio (examples wired)
- **StateTree:** `StateTreeComponent::patrol_chase()` на sentinels (Patrol ↔ Chase)
- **VFX:** `ParticleSystem` + `upload_bytes()` на dash/pickup (GPU sim + draw pass подключены в кадре)
- **Audio:** `with_audio` + `play_sfx("dash"|"pickup")` + mixer sync (no-op без устройства)

**Gate:** `cargo check --example prism_vault -p redox_core` — ✅  
**Gate:** `cargo check --workspace` — см. Verification

---

## ⚠️ Хвосты (polish, вне обязательного каркаса v2)

- [ ] DDGI GPU ray-tracing / visibility (сейчас CPU probe fill + GPU sample)
- [ ] SDF page table ← scene streaming (сейчас baked demo bricks)
- [x] Substrate lobes в VG resolve
- [ ] Полный Substrate lobe graph
- [x] GPU particle draw/compute pass
- [x] Skeletal animation: pose sampling/blending, CPU skinning streamed into mesh slots, glTF skin import, two-bone IK (`redox_anim`, `skinned_demo`)
- [x] Editor AnimGraph drives real keyframed clips with a live pose preview
- [x] VSM: GPU page requests, LRU eviction, camera-following fine levels, 256 px pages (0.4 cm texels)
- [x] GPU timestamp profiler, golden-frame regression mode, headless rendering parity
- [x] sRGB tone-mapping fix, HDR environment upload fix, IBL intensity control
- [ ] GPU skinning, skinned normal maps
- [ ] Revive or remove the CSM path (shadows are VSM-only; CSM renders everything in shadow)
- [ ] MetaSounds graph beyond bus routing (named graphs / modulators)
- [ ] Editor / Godot-like UI — **не в scope Uplift v2** (OFPA = format + runtime + CLI)

---

## Verification

```bash
cargo check --workspace
cargo check --example prism_vault -p redox_core
cargo run --example showcase -p redox_core
cargo run --example prism_vault -p redox_core
# optional world regen:
cargo run -p redox_scene --bin pcg -- --out worlds/arena --kind arena --width 6 --height 6 --cell-size 16 --seed 42
```

**Prism Vault** (`examples/prism_vault.rs`): сбор кристаллов, StateTree-стражи, dash, VG-кристаллы, World Partition streaming, ParticleSystem, `play_sfx`, F1 debug panel с честным статусом TSR / VSM / ScatterLight / DDGI / SDF / Bloom / Substrate / VG / WorldPartition / StateTree.
