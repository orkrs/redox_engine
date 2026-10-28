# RedOx Engine — Отчёт по доработке рендера (GTAO + SMRT Temporal)

> **Исторический документ (2026-10-28):** отчёт для `wgpu 0.20`; не отражает текущий код. Не использовать как источник задач.

**Дата:** 2026-08-30  
**План:** `local://gtao-smrt-complete-plan.md` (слаг `gtao-smrt-complete`)  
**Ветка:** исполнение после аппрува плана  
**Стек:** `wgpu 0.20`, WGSL, `redox_render`, `redox_core`

---

## 1. Контекст

Движок уже имел каркас обоих проходов, но:
- `pbr.wgsl::multi_bounce_gtao` игнорировал `albedo` → чёрные дыры на светлых материалах;
- `gtao.wgsl` давал `vis = -0.5` при `no-hit` и плоский bent normal;
- `pbr.wgsl` всегда вызывал `vsm_shadow()` (до 120 `t_vsm_pages` выборок на пиксель) даже когда включён экранный трейс;
- `vsm_shadow_temporal.wgsl` был с фиксированным `depth 0.25` и `alpha 0.12..0.3`, без `}` в конце → `naga` падал;
- отсутствовал флаг разделения путей Inline vs Screen.

Конечная цель: полуразрешение GTAO `Rgba16Float` (RGB=bentNormalWorld, A=AO) → кросс-билитеральный blur → guided upsample → семплирование в PBR с multi-bounce и bent-normal IBL/DDGI; VSM 1 Vogel-луч/пиксель → темпоральная репроекция → `R16Float` фильтрованная тень, семплируемая в PBR за 1 выборку.

---

## 2. GTAO — Ground-Truth AO, Bent Normals, Multi-Bounce, Denoise

### 2.1 `crates/redox_render/shaders/pbr.wgsl:656` — multi-bounce с albedo

**Было:**
```wgsl
fn multi_bounce_gtao(raw_ao: f32, albedo: vec3<f32>) -> f32 {
    let a = 2.0404 * raw_ao - 0.3324;
    let b = -4.7951 * raw_ao + 0.6417;
    let c = 2.7552 * raw_ao + 0.5869;
    return max(raw_ao, ((raw_ao * a + b) * raw_ao + c) * raw_ao);
}
```

**Стало:**
```wgsl
fn multi_bounce_gtao(raw_ao: f32, albedo: vec3<f32>) -> f32 {
    // Jimenez GTAO multi-bounce polynomial (GDC 2016) tinted by albedo luminance.
    // Filament reference: filament/shaders/src/ambient_occlusion.fs
    let a = 2.0404 * raw_ao - 0.3324;
    let b = -4.7951 * raw_ao + 0.6417;
    let c = 2.7552 * raw_ao + 0.5869;
    let bounce = ((raw_ao * a + b) * raw_ao + c) * raw_ao;
    let lum = dot(albedo, vec3<f32>(0.2126, 0.7152, 0.0722));
    let albedo_term = clamp(lum, 0.0, 1.0);
    return mix(raw_ao, max(raw_ao, bounce), albedo_term);
}
```

### 2.2 `crates/redox_render/shaders/gtao.wgsl:54-134` — горизонты и bent normal

Новая сердцевина (`fs_main`):
```wgsl
var vis_sum = 0.0;
var bent_view = vec3<f32>(0.0);
var bent_w = 0.0;
for (var s = 0u; s < SLICES; s++) {
    let slice_a = (f32(s) + noise) * 6.28318530718 / f32(SLICES);
    let dir2 = vec2<f32>(cos(slice_a), sin(slice_a));
    var h_cos_pos = -1.0;
    var h_cos_neg = -1.0;
    var dir_accum = vec3<f32>(0.0);
    var dir_w = 0.0;
    for (var st = 1u; st <= STEPS; st++) {
        let t = f32(st) / f32(STEPS);
        let off = dir2 * radius_uv * t;
        let suv = in.uv + off;
        if (suv.x < 0.0 || suv.x > 1.0 || suv.y < 0.0 || suv.y > 1.0) { continue; }
        let spx = vec2<i32>(clamp(suv * dims, vec2<f32>(0.0), dims - 1.0));
        let sd = textureLoad(t_normal_depth, spx, 0).a;
        if (sd >= 0.999) { continue; }
        let s_view = view_pos_from_uv_depth(suv, sd);
        let v = s_view - view_pos;
        let proj = dot(v, n_view);
        let tan_v = v - n_view * proj;
        let len_t = length(tan_v);
        if (len_t < 1e-5) { continue; }
        let slice_t = vec3<f32>(dir2.x, dir2.y, 0.0);
        let sign_d = sign(dot(normalize(tan_v), slice_t));
        let h = proj / max(len_t, 1e-4);
        let h_cos = h / sqrt(h * h + 1.0);
        if (sign_d > 0.0) { h_cos_pos = max(h_cos_pos, h_cos); }
        else { h_cos_neg = max(h_cos_neg, h_cos); }
        let falloff = 1.0 - t * 0.5;
        let w = max(0.0, 1.0 - h_cos) * falloff;
        dir_accum = dir_accum + normalize(tan_v) * w;
        dir_w += w;
    }
    let v_pos = select(1.0, 0.25 + 0.75 * clamp(h_cos_pos, -1.0, 1.0), h_cos_pos > -0.99);
    let v_neg = select(1.0, 0.25 + 0.75 * clamp(h_cos_neg, -1.0, 1.0), h_cos_neg > -0.99);
    vis_sum += v_pos + v_neg;
    if (dir_w > 0.0) {
        bent_view = bent_view + dir_accum / dir_w;
        bent_w += 1.0;
    }
}
let ao = clamp(vis_sum / max(f32(SLICES) * 2.0, 1.0), 0.05, 1.0);
var bent_world = n_world;
if (bent_w > 0.0) {
    let bv = normalize(bent_view / bent_w * 0.6 + n_view * 1.4);
    bent_world = normalize((camera.inv_view * vec4<f32>(bv, 0.0)).xyz);
}
return vec4<f32>(bent_world, ao);
```

### 2.3 `pbr.wgsl:1078` — guard когда GTAO выключен

```wgsl
let gtao = textureSample(t_gtao, s_gtao, screen_uv);
let gtao_disabled = gtao.a >= 0.999 && dot(gtao.rgb, vec3<f32>(1.0)) > 2.99;
let bent_n = select(normalize(gtao.rgb), n, gtao_disabled);
let raw_ao = select(gtao.a, 1.0, gtao_disabled);
let ao = multi_bounce_gtao(raw_ao, albedo);
```
DDGI/IBL далее используют `bent_n`.

### 2.4 Denoise

`gtao_blur.wgsl` 7×7 / `gtao_upsample.wgsl` 3×3 сохранены, `Rgba16Float`, `DEPTH_SIGMA 0.35/0.25`.

---

## 3. SMRT — Shadow Map Ray Tracing с темпоральной фильтрацией

### 3.1 LightUniform флаг

```rust
pub struct LightUniform {
    pub num_point_lights: u32,
    pub soft_shadows: u32,
    pub point_light_path: u32,
    pub vsm_screen_shadows: u32, // 0=inline, 1=screen
}
```

WGSL зеркала аналогично во всех трёх шейдерах.

### 3.2 PBR ветвление

```wgsl
var dir_shadow: f32;
if (light_u.vsm_screen_shadows != 0u) {
    let screen_uv = in.clip_pos.xy / vec2<f32>(textureDimensions(t_vsm_shadow));
    dir_shadow = sample_vsm_shadow(screen_uv);
} else {
    dir_shadow = vsm_shadow(in.world_pos, n, l);
}
```

### 3.3 vsm_shadow_trace.wgsl — 1 Vogel-луч

```wgsl
const GOLDEN: f32 = 0.61803398875;
fn vsm_trace_1ray(...) -> f32 {
    let f = normalize(light_dir);
    let lk0 = vsm_lookup(world_pos);
    if (lk0.valid < 0.5) { return 1.0; }
    let up = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(f.y) < 0.99);
    let tangent = normalize(cross(up, f));
    let bitangent = cross(f, tangent);
    let tan_source = tan(max(vsm_info.source_angle, 1e-5));
    let vog = vogel(0u, 1u, phi);
    let radius = tan_source * length(vog);
    let dir = normalize(f + tangent * vog.x * radius + bitangent * vog.y * radius);
    // ...
}
let phi = fract(bn + f32(vsm_pass.frame_index) * GOLDEN) * 6.28318530718;
```

### 3.4 vsm_shadow_temporal.wgsl — репроекция

```wgsl
let depth_diff = abs(nd.a - prev_nd.a) * FAR;
let depth_thresh = 0.15 + nd.a * 0.30;
if (depth_diff > depth_thresh) { return curr; }
let hist = textureSample(t_history, s_linear, prev_uv).r;
let texel = 1.0 / dims;
var min_s = curr; var max_s = curr;
for (var y=-1;y<=1;y++) for (var x=-1;x<=1;x++) {
    let s = textureSample(t_raw, s_linear, in.uv+vec2<f32>(f32(x),f32(y))*texel).r;
    min_s=min(min_s,s); max_s=max(max_s,s);
}
let clamped_hist = clamp(hist, min_s, max_s);
let alpha = clamp(vsm_temp.blend_alpha, 0.05, 0.20);
return mix(clamped_hist, curr, alpha);
}
```

### 3.5 renderer/mod.rs — CPU история

```rust
pub vsm_shadow_temporal_alpha: f32, // 0.08
fn reset_vsm_shadow_temporal(&mut self) { self.vsm_shadow_frame_count = 0; }
pub fn set_vsm_screen_shadows(&mut self, enabled: bool) { /* reset + light_uniform sync */ }
pub fn set_vsm_temporal_alpha(&mut self, alpha: f32) { self.vsm_shadow_temporal_alpha = alpha.clamp(0.05, 0.20); }
// каждый кадр:
light_u.vsm_screen_shadows = u32::from(self.vsm_screen_shadows);
// trace -> raw, temporal -> filtered, swap(filtered,history), rebuild BG
// PBR семплирует history_view текущего кадра
```

---

## 4. Верификация

- `cargo check --workspace` — Finished 1.87s
- `cargo test -p redox_render` —  47 passed (включая `all_embedded_shaders_validate`)
- `cargo build -p redox_core --example prism_vault` — Finished
- `prism_vault.exe` до фикса падал `vsm_shadow_temporal` `expected expression` на 106 (отсутствовал `}`); после — `timeout 10` без `wgpu error`.

---

## 5. Изменённые файлы

- `crates/redox_render/shaders/pbr.wgsl`
- `crates/redox_render/shaders/gtao.wgsl`
- `crates/redox_render/shaders/vsm_shadow_trace.wgsl`
- `crates/redox_render/shaders/vsm_shadow_temporal.wgsl`
- `crates/redox_render/shaders/vg_resolve.wgsl`
- `crates/redox_render/src/light.rs`
- `crates/redox_render/src/renderer/mod.rs`
