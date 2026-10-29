# PST-06 — Туман: высотный (аналитический), затем объёмный со светом солнца (C5)

- **Размер:** M (шаг 1) + S (шаг 2) + L (шаг 3, только по решению владельца). **Крейт:** `redox_render`. **Зависит от:** PST-01 (`PostFx`, готово). **Блокирует:** ничего.
- **Модель:** haiku для шагов 1–2; шаг 3 — после показа результата шагов 1–2 владельцу.
- **По умолчанию выключено; выключенное не меняет кадр побитно.**

## Суть и аудит
Тумана в движке **нет вообще**: `grep -rin fog crates/redox_render/src crates/redox_render/shaders` ничего не находит. План (`PLAN_LIGHTING.md` §4) описывает froxel-инъекцию + интегрирование + композицию.
Это три новых прохода и 3D-текстура; чтобы не строить всё сразу, спека идёт от дешёвого к дорогому. Шаги 1–2 уже дают видимый эффект (глубина сцены, «воздух»); шаг 3 нужен только ради световых лучей через тени.

## Что уже есть (проверено)
- Каркас пост-цепочки `PostFx` (`post/post_fx.rs`): два Rgba16Float-буфера ping-pong, `run_chain`, tone-bind-groups, пересоздание в `resize.rs`; эффекты — motion blur (`shaders/motion_blur.wgsl`), DoF (`shaders/dof.wgsl`). DoF читает **глубину** (`texture_depth_2d`, стандартный Z 0..1) и линеаризует `near*far/(far - d*(far-near))` — тот же приём годится для тумана.
- Данные камеры: `CameraUniform` (`camera.rs:79–95`): `view_proj`, `camera_pos`, `inv_proj`, `prev_view_proj`. **Обратной view_proj нет** — для мировой позиции пикселя считать на CPU и класть в uniform эффекта (**ПРОВЕРИТЬ**, как `update_camera_buffer` получает матрицы: `grep -n "fn update_camera_buffer" -A 25 crates/redox_render/src/renderer/*.rs`).
- Солнце: `DirectionalLight` (`light.rs`): направление, цвет, интенсивность; в кадре — `LightScene` и `light_uniform`. VSM-тени солнца: функция `vsm_shadow()` в `pbr.wgsl`, привязка 24 (только для шага 3).
- TAA-ветка единственная, где работает `PostFx` (как motion blur/DoF): в режимах без TAA туман не работает — документировать в `set_fog`.

## Чистая математика (общая для шагов, `post/fog.rs`, юнит-тесты без GPU; зеркало в WGSL)
```rust
/// Height fog: density falls off exponentially with height. Defaults: density 0.02 /m at `base_height`, falloff 0.15 /m.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FogSettings {
    pub color: [f32; 3],         // ambient (sky) colour of the fog
    pub density: f32,            // extinction per metre at base_height
    pub height_falloff: f32,     // 1/m; 0 = uniform fog
    pub base_height: f32,        // world y where density == `density`
    pub start_distance: f32,     // no fog closer than this
    pub max_opacity: f32,        // cap of the fog opacity, 0..1 (default 0.95)
    pub sun_scatter: f32,        // 0..1: strength of the sun in-scatter (shader step 2); 0 = off
    pub anisotropy: f32,         // Henyey-Greenstein g, -0.9..0.9 (default 0.5)
}
/// Optical depth along a ray: integral of density0 * exp(-falloff * (y - base)) over `distance` metres, starting at height `y0`,
/// rising by `dy_per_m` per metre of path. Analytic; falls back to density * distance when the exponent is ~0.
pub fn optical_depth(s: &FogSettings, y0: f32, dy_per_m: f32, distance: f32) -> f32;
/// Fraction of light that reaches the eye: exp(-optical_depth).
pub fn transmittance(optical_depth: f32) -> f32;
/// Henyey-Greenstein phase function value for cos(theta) between the view ray and the direction towards the sun.
pub fn hg_phase(cos_theta: f32, g: f32) -> f32;
/// Depth (metres) of froxel slice `i` of `n`, exponentially distributed between `near` and `far` (used in step 3).
pub fn slice_depth(i: u32, n: u32, near: f32, far: f32) -> f32;
```
Формулы:
- `k = height_falloff * dy_per_m`; если `|k| < 1e-4`: `tau = density * exp(-height_falloff*(y0-base)) * distance`; иначе `tau = density * exp(-height_falloff*(y0-base)) * (1 - exp(-k*distance)) / k`.
- Расстояние учитывает `start_distance`: считается `max(distance - start, 0)`, `y0` сдвигается вдоль луча на `start`.
- `hg_phase(c, g) = (1 - g²) / (4π (1 + g² - 2 g c)^1.5)`.
- `slice_depth(i, n, near, far) = near * (far/near)^(i/n)`.

Юнит-тесты (`post/fog.rs`):
```rust
#[test] fn uniform_fog_is_density_times_distance() { /* falloff 0: optical_depth == density * distance */ }
#[test] fn fog_thins_with_height() { /* ray going up has less depth than a horizontal one at the same base */ }
#[test] fn the_analytic_integral_matches_a_numeric_sum() { /* 1000 steps of a rising ray, relative error < 1e-3 */ }
#[test] fn a_near_horizontal_ray_does_not_divide_by_zero() { /* dy_per_m = 1e-7 -> finite and ~ horizontal value */ }
#[test] fn transmittance_is_one_without_fog_and_falls_with_depth() { assert_eq!(transmittance(0.0), 1.0); assert!(transmittance(2.0) < transmittance(1.0)); }
#[test] fn hg_phase_is_forward_peaked_for_positive_g_and_integrates_to_about_one() { /* sum over sphere by 2000 cos steps *2π ≈ 1 ± 0.02 */ }
#[test] fn hg_phase_with_g_zero_is_isotropic() { /* hg_phase(c, 0) == 1/(4π) for several c */ }
#[test] fn start_distance_removes_fog_close_to_the_camera() { /* distance < start -> 0 */ }
#[test] fn slice_depths_are_increasing_and_hit_the_ends() { /* slice_depth(0)==near, (n)==far, strictly increasing */ }
```

## Шаг 1 — высотный туман как пост-эффект (M)
- Шейдер `shaders/fog.wgsl` (шаблон — `dof.wgsl`: тот же полноэкранный треугольник, бинды: `0` цвет, `1` глубина `texture_depth_2d`, `2` sampler, `3` uniform).
  Uniform: `FogSettings` + `near, far, camera_pos, inv_view_proj (mat4), screen`.
  Для пикселя: линейная дистанция вдоль **луча** (не по оси Z): `dist = linear_z / dot(ray_dir, forward)`; мировой луч `ray_dir` из `inv_view_proj` (две точки near/far); `tau = optical_depth(...)`; `t = transmittance(tau)`;
  `out = mix(fog_color, scene, max(t, 1 - max_opacity))`. Небо (`depth == 1.0`) получает туман как на дистанции `far`, но с множителем 0,5 (чтобы горизонт сливался, а небо не белело целиком).
- `PostFx`: третий эффект в `run_chain` после DoF (последним: туман ложится поверх размытия); `Renderer::set_fog(Option<FogSettings>)` / `fog()`; пересоздание bind group при ресайзе — так же, как DoF.
- Тесты GPU (`tests/fog.rs`, пропуск без адаптера; сцена — как в `tests/post_fx.rs`): (а) `with_fog_off_the_frame_is_unchanged` (побитно); (б) `fog_pulls_distant_pixels_towards_the_fog_colour` — куб на 50 м и такой же на 5 м, плотность 0,05: далёкий ближе по цвету к `fog.color`, чем близкий (метрика: расстояние в RGB, разница > 0,1); (в) `fog_with_zero_density_changes_nothing` (допуск 1/255); (г) `fog_survives_a_resize`.

## Шаг 2 — рассеяние солнца в тумане без теней (S)
- В uniform: `sun_dir` (куда светит, нормированный) и `sun_color * intensity` (из `LightScene`, **ПРОВЕРИТЬ** путь: `grep -n "directional" crates/redox_render/src/renderer/frame.rs | head`).
- Цвет тумана: `fog_color = ambient_color + sun_scatter * sun_color * hg_phase(dot(view_dir, -sun_dir), g) * (1 - t)`; без теней, поэтому «лучи» видны и сквозь стены — документировать ограничение.
- Тест: `sun_scatter_brightens_fog_towards_the_sun` — камера смотрит на солнце и от солнца, `sun_scatter` 1: средняя яркость тумана в кадре «на солнце» больше. Юниты — `hg_phase` (уже есть).

## Шаг 3 — froxel-сетка с тенями VSM (L; перед стартом — согласие владельца)
Условие: шаги 1–2 приняты, и владелец хочет лучи через тени (окна, деревья).
1. 3D-текстура `Rgba16Float` 160×90×64 (инъекция: рассеяние рgb + экстинкция a); глубины слоёв `slice_depth(i, 64, near, min(far, 120))`.
2. Compute `fog_inject.wgsl`: для ячейки — мировая позиция из `inv_view_proj` + слой, плотность из `optical_depth`-модели, свет солнца × `vsm_shadow(world_pos)` × `hg_phase` (подключить функцию VSM из `pbr.wgsl`: **ПРОВЕРИТЬ** вынос в общий WGSL-модуль или копия; привязки VSM — таблица страниц, пул).
3. Compute `fog_integrate.wgsl`: накопление по Z (front-to-back) → `(inscatter, transmittance)` в той же текстуре.
4. Композиция вместо аналитического шага 1: `scene * T + inscatter` с трилинейной выборкой по `(uv, slice(depth))`.
5. Временная фильтрация (джиттер по Z и смешивание с прошлым кадром) — **не** в первой версии.
6. Тесты: юниты `slice_depth`; GPU — сравнение с аналитическим туманом при выключенных тенях (допуск 0,05), луч света через «окно» (просвет в блокирующем кубе) ярче, чем в тени (метрика: яркость пикселей внутри/вне).
Бюджет: ≤ 0,8 мс на 1080p (замер в отчёт).

## Границы
- **Читать:** `post/post_fx.rs` (целиком), `shaders/dof.wgsl`, `renderer/frame/post.rs`, `renderer/{init,resize,settings}.rs`, `camera.rs:75–130`, `light.rs`, `tests/post_fx.rs`; для шага 3 — `shaders/pbr.wgsl` (`vsm_shadow`), `shaders/vsm_visibility.wgsl`.
- **Создать:** `post/fog.rs`, `shaders/fog.wgsl`, `tests/fog.rs` (шаг 3: `shaders/fog_inject.wgsl`, `shaders/fog_integrate.wgsl`).
- **Менять:** `post/mod.rs`, `post/post_fx.rs`, `renderer/{mod,init,resize,settings}.rs`, `renderer/frame/post.rs`.
- **Не трогать:** `pbr.wgsl` (кроме выноса VSM-функции на шаге 3 — обсудить), TAA, тени.

## Проверка
`cargo fmt`, `cargo clippy -p redox_render --all-targets -- -D warnings`, `cargo test -p redox_render --lib fog`, `cargo test -p redox_render --test fog --test post_fx --test golden_frames --test resize_bloom`, `python scripts/check_file_sizes.py`, `python scripts/gen_crates_doc.py --check`.
Живая проверка владельцем: Foxfire с `set_fog(Some(default))`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Эталоны не обновлять. Шаг 3 без явного «да» координатора не начинать.
