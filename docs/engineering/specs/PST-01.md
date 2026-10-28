# PST-01 — Размытие движения (motion blur) и каркас цепочки пост-эффектов

- **Размер:** M. **Крейт:** `redox_render`. **Зависит от:** ничего (скорость объектов A1 готова). **Блокирует:** PST-02.
- **Модель:** sonnet (шейдер, пайплайн, пересборка bind group при ресайзе; чистая математика покрыта юнит-тестами, GPU — тестом сравнения).
- **Параллельно с:** ничем из `renderer/` (правит `init.rs`, `resize.rs`, `mod.rs`, `frame/post.rs`) — RND-04 и RND-05 запускать в другое время.

## Суть
Есть вектора скорости (`taa_velocity_tex`, `Rg16Float`-подобная текстура: xy = смещение в UV, камера + движущиеся объекты, `record_object_velocity`), но motion blur нет.
Скорость считается **только в TAA-ветке** `record_post_chain` (`renderer/frame/post.rs`, `if self.taa_enabled { … }`), поэтому и эффект работает в ней. Ветки «sharp AA» и «стандартная» не
затрагиваются (эффект там выключен; задокументировать в `set_motion_blur`).

## Как устроен хвост кадра (прочитано в коде, `post.rs`)
`TAA-проход → taa_history[history_write_idx]`; затем `record_bloom_chain(encoder, taa_out_view)`; затем `tone_mapping_pass` с готовым bind group
`self.taa_tone_bgs[history_write_idx]` (создан `create_tonemap_bind_group(device, pipeline, view, bloom_mip_views[0], sampler, uniform)` в `init.rs:560–573`; пересоздаётся
в `resize.rs:285` и `332`). История TAA **не должна** содержать размытие (иначе оно накапливается), поэтому блюр пишет в отдельную текстуру, а tone mapping читает её через
собственный bind group.

## Границы
- **Читать:** `renderer/frame/post.rs` (целиком), `init.rs:530–575` (создание velocity/history/tone bg), `resize.rs:255–340` (пересборка), `post/tone_mapping.rs`
  (`create_tonemap_bind_group`), `pass/msaa_resolve.rs` (190 строк — образец полноэкранного прохода: pipeline, layout, bind group), `shaders/taa.wgsl` (вершинный шейдер
  полноэкранного треугольника), `tests/object_velocity.rs` (как собрать кадр с движущимся объектом), `renderer/settings.rs` (стиль сеттеров).
- **Создать:** `src/post/post_fx.rs`, `shaders/motion_blur.wgsl`, `tests/motion_blur.rs`.
- **Менять:** `post/mod.rs` (`mod post_fx;`), `renderer/mod.rs` (поля), `renderer/init.rs`, `renderer/resize.rs`, `renderer/frame/post.rs`, `renderer/settings.rs`, `frame_graph.rs` (**не** менять: эффект внутри пасса `taa_tonemap_bloom_smaa`).
- **Не трогать:** TAA-шейдер, bloom, SMAA, golden-эталоны (при выключенном эффекте картинка обязана совпасть **побитно**).

## Решение
### 1. Чистая математика (`post/post_fx.rs`, юнит-тесты без GPU)
```rust
/// Parameters of the motion blur.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionBlurSettings {
    /// Fraction of the frame time the virtual shutter stays open: 0.5 = 180 degree shutter.
    pub shutter: f32,
    /// Longest blur in pixels (the tap offsets are scaled down to it).
    pub max_pixels: f32,
    /// Number of taps along the blur direction (at least 2).
    pub taps: u32,
}
impl Default for MotionBlurSettings { /* shutter 0.5, max_pixels 24.0, taps 8 */ }

/// UV offsets of the taps for a pixel with the given velocity (UV per frame). Symmetric around 0, spread over
/// `velocity * shutter`, the total length limited to `max_pixels` on a target of `screen` pixels.
/// Returns an empty list when the blur would be shorter than half a pixel (the pixel is left sharp).
pub fn motion_blur_offsets(velocity_uv: [f32; 2], screen: [f32; 2], s: &MotionBlurSettings) -> Vec<[f32; 2]>;
```
Тот же алгоритм записывается в WGSL; в комментариях шейдера и функции — «держать в синхроне». Формулы:
`v = velocity_uv * shutter`; `len_px = length(v * screen)`; если `len_px < 0.5` → пусто; если `len_px > max_pixels` → `v *= max_pixels / len_px`;
`offset_i = v * (i / (taps - 1) - 0.5)` для `i in 0..taps`.

### 2. Ресурсы и состояние (`PostFx`)
```rust
pub(crate) struct PostFx {
    pub textures: [wgpu::Texture; 2],       // Rgba16Float, internal render size (as taa_history), RENDER_ATTACHMENT | TEXTURE_BINDING
    pub views: [wgpu::TextureView; 2],
    pub tone_bgs: [wgpu::BindGroup; 2],     // create_tonemap_bind_group over views[i] + bloom_mip_views[0]
    pub motion_blur: MotionBlurPass,        // pipeline + bind group layout + uniform buffer
}
```
Две текстуры нужны, чтобы PST-02 (DoF) мог идти по цепочке «A→B» без нового каркаса. Метод
`fn run_chain(&self, encoder, source: &TextureView, velocity: &TextureView, settings: &PostFxSettings) -> Option<usize>`
возвращает индекс текстуры с итогом или `None`, если все эффекты выключены (tone mapping тогда берёт `taa_tone_bgs[history_write_idx]` как раньше).
Размер как у `taa_history` (выходной размер, а не `internal_*`); пересоздание и `tone_bgs` — в тех же местах `resize.rs`, где пересоздаются `taa_tone_bgs` (обе ветки, строки ~285 и ~332).

### 3. Шейдер (`shaders/motion_blur.wgsl`)
Полноэкранный треугольник (как в `taa.wgsl`), биндинги: `0` текстура цвета (после TAA), `1` текстура скорости (`taa_velocity_view`; формат фильтруемый ли — **проверить**, при нефильтруемом
читать через `textureLoad`), `2` sampler linear clamp, `3` uniform `{ screen: vec2<f32>, shutter: f32, max_pixels: f32, taps: u32, _pad: vec3<u32> }`.
`fs_main`: `v = textureLoad(velocity, pixel).xy`; цикл по тапам как в `motion_blur_offsets`; среднее цветов; если `len_px < 0.5` — вернуть исходный цвет без выборок по тапам.
Скорость берётся в **текущем** пикселе (простое решение; артефакты на границах объектов допустимы и не чинятся здесь).

### 4. Управление
`Renderer::set_motion_blur(&mut self, settings: Option<MotionBlurSettings>)` и `motion_blur(&self) -> Option<MotionBlurSettings>`; по умолчанию `None` (выключено).
В `post.rs`, после TAA-прохода (`6d`) и до `record_bloom_chain`: `let fx = self.post_fx.run_chain(...)`; если `Some(i)` — bloom строится от `post_fx.views[i]`, tone mapping использует `post_fx.tone_bgs[i]`; иначе как сейчас.

## Пошагово
1. Юнит-тесты математики (ниже) → красные → `motion_blur_offsets`.
2. Шейдер + `MotionBlurPass` + `PostFx` + поля/инициализация/ресайз.
3. Подключение в `post.rs` и `set_motion_blur`.
4. GPU-тест, затем golden-регрессия: `cargo test -p redox_render --test golden_frames --test object_velocity --test resize_bloom`.

## Тесты
Юнит (в `post/post_fx.rs`):
```rust
#[test] fn a_still_pixel_gets_no_taps() {
    assert!(motion_blur_offsets([0.0, 0.0], [1920.0, 1080.0], &MotionBlurSettings::default()).is_empty());
    // half a pixel of velocity * shutter 0.5 -> 0.25 px: still sharp
    assert!(motion_blur_offsets([0.5 / 1920.0, 0.0], [1920.0, 1080.0], &MotionBlurSettings::default()).is_empty());
}
#[test] fn taps_are_symmetric_and_as_many_as_requested() {
    let s = MotionBlurSettings { taps: 8, ..Default::default() };
    let o = motion_blur_offsets([0.02, 0.0], [1920.0, 1080.0], &s);
    assert_eq!(o.len(), 8);
    for i in 0..4 { assert!((o[i][0] + o[7 - i][0]).abs() < 1e-7); }
    assert!(o[0][0] < 0.0 && o[7][0] > 0.0);
}
#[test] fn the_blur_length_is_limited_to_max_pixels() {
    let s = MotionBlurSettings { shutter: 1.0, max_pixels: 10.0, taps: 5 };
    let o = motion_blur_offsets([0.5, 0.0], [1000.0, 1000.0], &s);   // would be 500 px
    let span_px = (o[4][0] - o[0][0]) * 1000.0;
    assert!((span_px - 10.0).abs() < 1e-3, "span {span_px}");
}
#[test] fn the_direction_follows_the_velocity() {
    let o = motion_blur_offsets([0.0, 0.03], [1000.0, 1000.0], &MotionBlurSettings::default());
    assert!(o.iter().all(|t| t[0].abs() < 1e-9));
}
#[test] fn fewer_than_two_taps_are_raised_to_two() {
    let s = MotionBlurSettings { taps: 0, ..Default::default() };
    assert_eq!(motion_blur_offsets([0.05, 0.0], [1000.0, 1000.0], &s).len(), 2);
}
```
GPU (`tests/motion_blur.rs`, пропуск без адаптера; сцена и кадр — по `tests/object_velocity.rs`; TAA включён):
```rust
#[test] fn with_the_effect_off_the_frame_is_unchanged() {
    // render a scene with a moving cube twice (two renderers, same inputs), one never calls set_motion_blur: pixels identical.
    // Then set_motion_blur(None) explicitly on the second: still identical.
}
#[test] fn a_moving_object_is_smeared_along_its_motion() {
    // cube moved by 0.4 units per frame along X for 3 frames (TAA on). Read the frame; count the pixels in a band around the cube
    // whose colour differs from the sky by > 0.05: with blur ON the covered width in pixels is at least 1.3x the width with blur OFF,
    // measured on the same row. Vertical extent is unchanged within 2 pixels.
}
#[test] fn a_still_scene_is_not_changed_by_the_effect() {
    // static cube, 4 frames, blur ON vs OFF: max per-channel difference <= 1/255 (velocity is zero, no taps).
}
#[test] fn the_effect_survives_a_resize() {
    // set_motion_blur(Some), render, resize to a different size, render again: no validation error (device error scope), image not black.
}
```

## Проверка
`cargo test -p redox_render --lib post_fx`, `cargo test -p redox_render --test motion_blur --test golden_frames --test object_velocity --test resize_bloom`, clippy `-p redox_render --all-targets`, fmt.
Оценка стоимости (в отчёт): время кадра с эффектом и без через GPU-профайлер renderer (если доступен) или число из RND-04; цель ≤ 0,4 мс на 1080p.
Живая проверка владельцем: покрутить камеру в Foxfire с включённым эффектом (после подключения в примере — отдельная мелкая задача).

## Для исполнителя
`git merge --ff-only main`. Если формат скорости не читается шейдером так, как написано, остановись и опиши фактический формат (`init.rs:537`). Golden-кадры не обновлять.
