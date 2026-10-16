# RedOx Engine — Комплексный план оптимизации и реконструкции изображения

> **Версия:** 2.0 (Engineering Architecture Design)  
> **Стек:** Rust, `wgpu 0.20`, WGSL, `glam 0.27`, Archetype ECS, Forward+ / Visibility Buffer (Meshlets)  
> **Цель:** Ликвидация CPU/GPU ботлнеков, переход к GPU-driven пайплайну и внедрение 4-уровневой системы темпоральной реконструкции (100% устранение ступенек, шума теней, мерцания бликов и пикселизации без потери резкости).

---

## 1. Архитектурная диаграмма целевого пайплайна кадра

```
                                          FRAME TIMELINE (1 ENCODER SUBMISSION / MULTI-THREADED RECORDING)
──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
[ CPU THREADS ]  ──> [ Rayon Task: ECS Graph ] ──> [ Rayon Task: Frustum/LOD ] ──> [ MT Command Recording (Pass Encoders) ]
                                                                                               │
──────────────────────────────────────────────────────────────────────────────────────────────┼───────────────────────────────────────────────────
[ GPU COMPUTE ]  ──> [ Hi-Z Downsample ] ──> [ Phase 1 Meshlet Cull ] ──> [ Light Culling ]   │  ──> [ Phase 2 Meshlet Cull ] ──> [ Particle Sim ]
                                                       │                                       │               │
──────────────────────────────────────────────────────────────────────────────────────────────┼───────────────────────────────────────────────────
[ GPU RENDER  ]                                        ▼                                       ▼               ▼
                 [ Shadow Atlas Batch / CSM ] ──> [ Depth/Normal Prepass ] ──> [ SSAO 0.5x ] ──> [ Main PBR Forward / VG Resolve ]
                                                                                                               │
                                                                                                               ▼
[ TSR RECONSTRUCT ] ──────────────────────────────────────────────────────────────────────────> [ TSR 2.0 Resolve + CAS Sharpen ] ──> [ Swapchain ]
```

---

## 2. Раздел I. Топ-10 архитектурных и графических оптимизаций

### Оптимизация 1. Перенос кластеризации света (Clustered Lighting) на GPU Compute

- **Текущее состояние:** Построение сетки $16 \times 16 \times 24$ (6144 кластера) и назначение до 512 источников выполняется на CPU каждый кадр (`clustering.rs`).
- **Техническая реализация:**
  1. **Compute-проход AABB (однократно/при изменении FOV):** Расчёт экранно-пространственных AABB кластеров в виде `array<ClusterAABB, 6144>` (`min_point`, `max_point` во view-space).
  2. **Compute-проход Light Culling (каждый кадр):** Воркгруппа `16x16x1` потоков. Общая память (`workgroupBarrier`) загружает источники света пачками, выполняет тест сфера-бокс и атомарно наполняет выходной `StorageBuffer` индексов света (`light_indices` + `cluster_metadata`).
- **Эффект:** Экономия 2–5 мс CPU-времени, устранение синхронного трансфера индексного буфера по PCIe.

---

### Оптимизация 2. Двухфазный GPU Occlusion Culling с Hi-Z пирамидой глубин

- **Текущее состояние:** В `VGSystem` реализован только Frustum + Backface-Cone culling на GPU; овердро на скрытой геометрии не отсекается.
- **Техническая реализация:**
  1. **Hi-Z Generation:** Построение пирамиды глубин из Depth Buffer предыдущего кадра через цепочку Compute-проходов (редукция $2\times 2$ с операцией `max(depth)` под конвенцию wgpu $[0, 1]$).
  2. **Phase 1 (Early Culling):** Тестирование Bounding Sphere мешлетов против Hi-Z прошлого кадра $\rightarrow$ запись прошедших индексов в `IndirectDrawBuffer` $\rightarrow$ рендер базового видимого пула.
  3. **Phase 2 (Late Culling):** Дорисовка Depth Prepass, обновление верхних мипов Hi-Z, проверка ранее отсечённых мешлетов $\rightarrow$ рендер вновь появившихся объектов вторым вызовом `draw_indexed_indirect`.
- **Эффект:** Сокращение овердро и числа отрисовываемых треугольников на 40–80% в насыщенных сценах.

---

### Оптимизация 3. GPU-driven симуляция частиц (Compute + Indirect Draw)

- **Текущее состояние:** `redox_vfx` выполняет интеграцию Эйлера, компактификацию массива и генерацию вершин (CPU billboard expansion) на процессоре с покадровым `queue.write_buffer`.
- **Техническая реализация:**
  1. Буферы частиц: два пинг-понг `StorageBuffer` структур `GpuParticle` (64 Б) + атомарный счётчик живых частиц `AtomicU32`.
  2. Симуляция: Compute Shader обновляет $P_{new} = P + V \cdot dt + 0.5 \cdot g \cdot dt^2$, декрементирует `life`, компактифицирует живые частицы и заполняет поля `DrawIndirectArgs` (`vertex_count = 4`, `instance_count = alive_count`).
  3. Отрисовка: Вершинный шейдер генерирует координаты квада по `vertex_index` ($0..4$) и считывает параметры инстанса по `instance_index` без передачи вершинных буферов с CPU.
- **Эффект:** 0 мс нагрузки на CPU при симуляции до 200 000+ частиц.

---

### Оптимизация 4. Рефакторинг шейдерных биндингов (Decoupled Bind Groups)

- **Текущее состояние:** В `pbr.wgsl` монолитная `group(0)` содержит **29 биндингов**, что упирается в лимиты драйверов и вызывает частые сбросы состояний.
- **Техническая реализация:** Разделение на 4 группы по частоте обновления:

| Bind Group  | Назначение      | Содержимое                                                       | Частота обновления |
| ----------- | --------------- | ---------------------------------------------------------------- | ------------------ |
| **Group 0** | Global / Frame  | CameraUniform, Global Samplers, Time, BRDF LUT, Blue Noise       | 1 раз за кадр      |
| **Group 1** | Pass / Scene    | CSM Array, VSM Clipmaps, Atlas, Clusters, DDGI Probes, SDF Atlas | 1 раз за пасс      |
| **Group 2** | Materials       | Storage-массивы дескрипторов материалов и связок текстур         | По мере изменения  |
| **Group 3** | Instance / Call | Model Storage, Dynamic Offsets                                   | На вызов/инстанс   |

- **Эффект:** Снижение оверхеда драйвера на перевалидацию дескрипторов, совместимость с мобильными и строгими Vulkan/Metal бекендами.

---

### Оптимизация 5. Параллельная запись командных буферов (Multi-Threaded Command Encoding)

- **Текущее состояние:** `Renderer::render_frame_into` (3360 строк) последовательно в один поток заполняет один `wgpu::CommandEncoder`.
- **Техническая реализация:**
  1. Декомпозиция конвейера на изолированные контексты проходов: `ShadowContext`, `PrepassContext`, `MainRenderContext`, `PostProcessContext`.
  2. Запись в пуле потоков `rayon`:
     ```rust
     let (shadow_cmd, prepass_cmd, main_cmd) = rayon::join(
         || record_shadow_passes(&device, &scene),
         || rayon::join(
             || record_depth_normal_prepass(&device, &scene),
             || record_main_pbr_pass(&device, &scene)
         )
     );
     queue.submit([shadow_cmd, prepass_cmd, main_cmd.0, main_cmd.1]);
     ```
- **Эффект:** Устранение простоя процессорных ядер на стадии подготовки кадра.

---

### Оптимизация 6. Параллельный диспетчер систем ECS на базе графа зависимостей

- **Текущее состояние:** `Dispatcher` в `redox_core` исполняет системы строго последовательно в основном потоке.
- **Техническая реализация:**
  1. Декларация доступа систем к ресурсам и компонентам (`Reads<A, B>`, `Writes<C>`).
  2. Построение ациклического графа зависимостей (DAG) стадий: непересекающиеся по мутациям системы исполняются параллельно через `rayon::scope`.
  3. Группировка: `PhysicsSync` $\parallel$ `AudioOcclusionRaycasts` $\parallel$ `Animation/StateTree`.
- **Эффект:** Полная утилизация многоядерных CPU в фазе симуляции.

---

### Оптимизация 7. Сжатие и квантование вершинных данных мешлетов

- **Текущее состояние:** `Vertex` занимает 44 байта (все компоненты в несжатом `f32`).
- **Техническая реализация:**
  - `Position`: Относительный `snorm16x4` внутри локального AABB мешлета $\rightarrow$ **8 байт** (вместо 12).
  - `Normal` и `Tangent`: Кодирование Octahedral mapping (`snorm8x2` для нормали + `snorm8x2` для тангента и знака $w$) $\rightarrow$ **4 байта** (вместо 24).
  - `UV`: `f16x2` (half2) $\rightarrow$ **4 байта** (вместо 8).
  - **Итог:** Сжатие вершины с **44 до 16 байт** (экономия 63.6%).
- **Эффект:** Сокращение требований к пропускной способности памяти (VRAM Bandwidth) в 2.75 раза на всех геометрических проходах (Shadow, Depth, Normal).

---

### Оптимизация 8. Half-Resolution SSAO и DDGI с Bilateral Upsample

- **Текущее состояние:** SSAO вычисляется в полном разрешении экрана с 64 сэмплами и размывается box-блюром 4x4.
- **Техническая реализация:**
  1. SSAO и сэмплирование DDGI выполняются в буфер $0.5 \times 0.5$ от внутреннего разрешения.
  2. Замена box-blur на **Depth/Normal-Aware Bilateral Blur** (кросс-билатеральный фильтр $5\times 5$).
  3. Апскейл в Full-Res через Guided Bilateral Filter с использованием Full-Res Normal/Depth текстур.
- **Эффект:** Ускорение прохода SSAO в 3.5–4 раза без визуальной потери резкости на гранях.

---

### Оптимизация 9. Батчинг теневых проходов атласа (Multi-Tile Array Rendering)

- **Текущее состояние:** Локальные тени в атласе 8192² рендерятся через цепочку сотен индивидуальных `render_pass` с переустановкой scissor/viewport.
- **Техническая реализация:**
  1. Объединение отрисовки всех кастеров в единый `render_pass` с `LoadOp::Load` для всего атласа.
  2. Передача массива матриц граней через Storage Buffer.
  3. Отрисовка через Indirect Instanced Draw с динамическим выбором Viewport/Scissor в вершинном шейдере или батчами по плиткам.
- **Эффект:** Сокращение сотен RenderPass-барьеров до одного прохода.

---

### Оптимизация 10. Ring-Buffered Staging Allocator для Uniform и Storage данных

- **Текущее Состояние:** Частые изолированные вызовы `queue.write_buffer` и точечные обновления буферов вызывают микроблокировки драйвера.
- **Техническая реализация:**
  1. Единый кольцевой `StagingBuffer` размером 64–128 МБ на 2 виртуальных кадра (`FRAMES_IN_FLIGHT = 2`).
  2. Все покадровые данные (трансформы, инстансы, скелеты, параметры материалов) пишутся последовательным линейным `memcpy` в единый блок хост-памяти.
  3. Синхронизация через один вызов `copy_buffer_to_buffer` в начале кадра.
- **Эффект:** Полная ликвидация просадок времени кадра (стабилизация 1% и 0.1% Low FPS).

---

## 3. Раздел II. Программа тотального устранения алиасинга и лесенок

Пайплайн ликвидации алиасинга охватывает 4 независимых источника искажений: геометрию, микрорельеф, тени и временную нестабильность.

```
       ИСТОЧНИК АЛИАСИНГА                       ПРИМЕНЯЕМЫЙ МЕТОД ЗАЩИТЫ
 ┌───────────────────────────────┐        ┌──────────────────────────────────────┐
 │ Ребра полигонов и геометрия   │ ─────> │ Subpixel Dilation + Halton-16 Jitter │
 ├───────────────────────────────┤        ├──────────────────────────────────────┤
 │ Тонкие объекты, трава, сетки  │ ─────> │ Stochastic A2C + Screen-Space Dilation│
 ├───────────────────────────────┤        ├──────────────────────────────────────┤
 │ Спекулярный шум и микрорельеф │ ─────> │ Geometric Specular AA + Toksvig Mips │
 ├───────────────────────────────┤        ├──────────────────────────────────────┤
 │ Зубцы и дрожание теней        │ ─────> │ Texel Snapping + SDF Contact Shadows │
 ├───────────────────────────────┤        ├──────────────────────────────────────┤
 │ «Мыло» и шлейфы в движении    │ ─────> │ YCoCg Karis Clamping + Reactive Mask │
 └───────────────────────────────┘        └──────────────────────────────────────┘
```

---

### Модуль 1. Геометрическое и субпиксельное сглаживание

#### 1.1. Subpixel Geometry Dilation для тонких объектов

- **Проблема:** Провода, прутья, растительность тоньше 1 пикселя распадаются на мерцающие пунктиры при удалении камеры.
- **Реализация в вершинном шейдере:**

  ```wgsl
  // Вычисление экранной толщины линии/границы
  let clip_pos = camera.view_proj * world_pos;
  let screen_size = vec2<f32>(camera.screen_width, camera.screen_height);
  let ndc_pos = clip_pos.xy / clip_pos.w;
  let line_dir_screen = normalize((next_clip_pos.xy / next_clip_pos.w - ndc_pos) * screen_size);
  let line_normal_screen = vec2<f32>(-line_dir_screen.y, line_dir_screen.x);

  let current_width_px = original_width_meters * screen_size.y / (clip_pos.w * 2.0 * tan(camera.fov * 0.5));
  let final_width_px = max(current_width_px, 1.0); // Никогда не тоньше 1 пикселя
  let coverage = current_width_px / final_width_px;

  // Экструзия вершины в экранном пространстве
  let offset_ndc = (line_normal_screen * (final_width_px / screen_size)) * clip_pos.w;
  out.position = vec4<f32>(clip_pos.xy + offset_ndc * sign_offset, clip_pos.z, clip_pos.w);
  out.coverage = coverage; // Передаётся во фрагментный шейдер для масштабирования альфы
  ```

#### 1.2. Stochastic Alpha-to-Coverage (A2C) с Blue Noise

- **Проблема:** Жесткий `discard` на листве и решетках порождает ступенчатые пиксельные края.
- **Реализация во фрагментном шейдере:**

  ```wgsl
  let blue_noise = textureSample(blue_noise_tex, point_sampler, screen_uv * noise_scale).r;
  let dither_threshold = fract(blue_noise + f32(globals.frame_index % 16u) * 0.61803398875);

  // Мягкое стохастическое отсечение, реконструируемое TAA в гладкий край
  if (albedo.a * in.coverage < dither_threshold) {
      discard;
  }
  ```

---

### Модуль 2. Шейдинг и микрорельеф (Specular & Normal AA)

#### 2.1. Geometric Specular AA (Защита от мерцания бликов)

- **Проблема:** Когда геометрическая кривизна полигона на пиксель превышает шероховатость ($Roughness \to 0$), NDF Кука-Торренса даёт деление на ноль, порождая одиночные яркие вспышки (fireflies).
- **Реализация в `pbr.wgsl`:**
  ```wgsl
  fn apply_geometric_specular_aa(normal: vec3<f32>, roughness: f32) -> f32 {
      let dndx = dpdx(normal);
      let dndy = dpdy(normal);
      let variance = max(dot(dndx, dndx), dot(dndy, dndy));
      let kernel_roughness = clamp(variance * 0.5, 0.0, 0.18);
      return sqrt(roughness * roughness + kernel_roughness);
  }
  ```

#### 2.2. Toksvig-фильтрация карт нормалей при генерации Mip-цепочки

- **Алгоритм в `redox_asset / mesh_loader`:** При переходе на более низкий Mip-уровень длина усреднённого вектора нормали уменьшается:
  $$\bar{N}_{len} = \|\sum_{i=1}^4 N_i\| \le 1.0$$
- Вычисляется дисперсия $\sigma^2 = \frac{1 - \bar{N}_{len}}{\bar{N}_{len}}$, которая добавляется в канал шероховатости соответствующего Mip-уровня. Это исключает появление шума на удалённых материалах.

---

### Модуль 3. Идеальные тени (Shadow Anti-Aliasing)

#### 3.1. Subpixel Texel Snapping для CSM каскадов

- **Реализация в `csm.rs`:** Центр ортографической проекции света жестко привязывается к шагу текселя в мировых координатах, полностью устраняя «плавание» и дрожание краев тени при перемещении камеры:
  ```rust
  let world_texel_size = (cascade_ortho_extent * 2.0) / shadow_map_resolution;
  light_view_proj.w_axis.x = (light_view_proj.w_axis.x / world_texel_size).floor() * world_texel_size;
  light_view_proj.w_axis.y = (light_view_proj.w_axis.y / world_texel_size).floor() * world_texel_size;
  ```

#### 3.2. Гибридная теневая маска (SDF Contact + VSM/PCSS Penumbra)

- **Реализация:**
  1. В радиусе $0.0..0.3\text{ м}$ от точки контакта тень рассчитывается через **SDF Cone Tracing** (дает субпиксельную резкость в точках соприкосновения без артефактов Shadow Acne).
  2. На расстоянии $> 0.3\text{ м}$ выполняется гладкий переход в **VSM + SMRT** (мягкая вариативная полутень).
  3. Смещение выборки диска фильтрации сэмплируется по Blue Noise паттерну и накапливается во времени через буфер истории теней.

---

### Модуль 4. Темпоральная реконструкция 2.0 (TSR / Temporal Super-Resolution)

#### 4.1. 16-фазный субпиксельный джиттер (Halton Sequence)

- Генерация субпиксельного смещения для проекционной матрицы:
  $$\Delta_{x, y} = (\text{Halton}(i, 2) - 0.5, \text{Halton}(i, 3) - 0.5) \cdot \frac{1.0}{\text{RenderResolution}_{x, y}}$$
  для $i \in [0..15]$.

#### 4.2. Алгоритм TSR Resolve (`taa.wgsl`)

```
 ┌────────────────────┐      ┌─────────────────────────┐
 │ Current HDR Sample │ ───> │ YCoCg-2020 Color Space  │ ──┐
 └────────────────────┘      └─────────────────────────┘   │
                                                           ▼
 ┌────────────────────┐      ┌─────────────────────────┐ ┌─────────────────────────────────┐
 │ Reprojected History│ ───> │ Catmull-Rom 5-Tap Fetch │─┤ Karis-Weighted Variance Clip    │
 └────────────────────┘      └─────────────────────────┘ └─────────────────────────────────┘
                                                           │
 ┌────────────────────┐      ┌─────────────────────────┐   │
 │ Reactive Mask (R8) │ ───> │ Adaptive Alpha Blend    │ <─┘
 └────────────────────┘      └─────────────────────────┘
                                   │
                                   ▼
                             ┌─────────────────────────┐
                             │ CAS Filter (Sharpening) │ ──> Clean Output
                             └─────────────────────────┘
```

1. **YCoCg-2020 Transformation & Karis Weighting:**  
   Перевод выборки $3\times3$ пикселей в цветовое пространство YCoCg. Вес каждого сэмпла масштабируется функцией подавления высококонтрастных пиков:
   $$w_i = \frac{1.0}{1.0 + \text{Luma}(C_i)}$$
2. **Variance Clipping:**  
   Вычисление среднего значения $\mu$ и среднеквадратичного отклонения $\sigma$ в окрестности $3\times3$. Ограничение истории боксом $[\mu - 1.25\sigma, \mu + 1.25\sigma]$.
3. **Reactive Mask Processing:**  
   Чтение вспомогательного буфера `ReactiveMask` (куда пишутся полупрозрачные частицы, эффекты и альфа-тест).  
   $$\alpha_{blend} = \text{mix}(\text{clamp}(0.05 + \frac{\|Velocity\|}{24.0}, 0.05, 0.20), 1.0, \text{ReactiveMaskValue})$$
4. **Contrast-Adaptive Sharpening (CAS) с защитой от Ringing:**  
   Финальный проход тонмаппинга выполняет 5-точечный крестообразный CAS-фильтр. Фильтр усиливает резкость текстур внутри поверхностей, но автоматически отключается на высококонтрастных гранях, предотвращая возникновение темных/светлых ореолов.

---

## 4. Дорожная карта внедрения и матрица трудоёмкости

| Фаза                            | Этап / Задача                                                                                                                 | Крейты                                 | Трудоёмкость   | Ожидаемый результат                                                                    |
| ------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- | -------------------------------------- | -------------- | -------------------------------------------------------------------------------------- |
| **Phase 1: Quick Wins**         | 1. Ring Staging Allocator<br>2. Half-Res SSAO + Bilateral<br>3. CAS Sharpening + YCoCg Clamping                               | `redox_render`                         | **1–2 недели** | Стабилизация фреймрейта (устранение фризов), прирост 15–20% FPS, исчезновение «мыла».  |
| **Phase 2: Core Shading & AA**  | 1. GPU Compute Light Culling<br>2. Разделение Bind Groups (0..3)<br>3. Geometric Specular AA + Texel Snapping                 | `redox_render`, `redox_math`           | **2–3 недели** | Снятие нагрузки с CPU (до +3–5 мс), полное устранение шума на бликах и дрожания теней. |
| **Phase 3: Geometry & Culling** | 1. Сжатие вершин мешлетов (44 $\to$ 16 Б)<br>2. Hi-Z Occlusion Culling (Phase 1 & 2)<br>3. Subpixel Dilation + Stochastic A2C | `redox_render`, `redox_asset`          | **3–4 недели** | Ускорение рендера геометрии в 2 раза, гладкие края растительности и тонких объектов.   |
| **Phase 4: Full Engine Scale**  | 1. GPU Compute Particles (Indirect Draw)<br>2. Multi-Threaded Command Recording<br>3. ECS Dependency Graph Dispatcher         | `redox_vfx`, `redox_core`, `redox_ecs` | **3–4 недели** | Полная масштабируемость движка на все ядра CPU и автономная симуляция VFX на GPU.      |

---

## 5. Итоговые метрики качества и производительности

После реализации плана RedOx Engine получает следующие характеристики:

- **CPU Overhead:** $\approx 0\text{ мс}$ на кластеризацию света и симуляцию частиц; масштабируемый параллельный пайплайн записи командных буферов.
- **VRAM Bandwidth:** Снижение трафика шины памяти на $\sim 60\%$ за счет квантования вершин и Hi-Z отсечения.
- **Качество изображения:**
  - **0% геометрических лесенок:** Субпиксельное покрытие Halton-16 + TSR.
  - **0% шума и вспышек на материалах:** Geometric Specular AA + Toksvig-фильтрация.
  - **0% мерцания теней:** Subpixel Texel Snapping + гибридные SDF/VSM контакты.
  - **0% потерь тонких деталей:** Сохранение проводов и растительности тоньше 1 пикселя через субпиксельную экструзию.

  Вот подборка самых авторитетных, современных и чистых **Open-Source эталонов** (на C++ и Rust), откуда берут шейдерный код, математику и архитектурные паттерны для AAA-движков.

Они разбиты строго по разделам нашего дизайн-плана.

---

### 1. Архитектура TSR, TAA 2.0, Reactive Mask и CAS (Антиалиасинг)

- **[AMD FidelityFX SDK (FSR 2 / FSR 3 / CAS)](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK)**
  - **Что смотреть:** Папки `sdk/include/FidelityFX/gpu/fsr2` и `cas`.
  - **Почему эталон:** Официальный открытый код индустриального стандарта темпорального апскейлинга. Содержит готовую реализацию:
    - 5-точечного Catmull-Rom с подавлением ringing;
    - YCoCg-2020 Karis variance clipping;
    - Автоматической генерации и обработки `Reactive Mask`;
    - Шейдера Contrast-Adaptive Sharpening (CAS).
- **[Playdead Temporal Reprojection (Inside TAA)](https://github.com/playdeadgames/temporal)**
  - **Что смотреть:** `Shaders/TemporalReprojection.shader`.
  - **Почему эталон:** Классический эталон чистого TAA, на котором построены почти все современные реализации (включая Unreal TSR и Unity HDRP). Идеально для переноса в WGSL.

---

### 2. Geometric Specular AA, Toksvig и честный PBR

- **[Google Filament](https://github.com/google/filament)**
  - **Что смотреть:** `shaders/src/brdf.fs`, `surface_shading.fs`, `light_indirect.fs`.
  - **Почему эталон:** Лучшая в мире открытая кодовая база по PBR-шейдингу с математическими выкладками.
  - **Что брать для плана:** Готовые функции для **Geometric Specular AA** (адаптация шероховатости к кривизне нормалей через `fwidth`/`dpdx`), Toksvig normal filtering и octahedral mapping.

---

### 3. GPU Occlusion Culling (Hi-Z), Meshlets и GPU-Driven Rendering

- **[Wicked Engine](https://github.com/turanszkij/WickedEngine)**
  - **Что смотреть:** `WickedEngine/shaders/meshlet_*.hlsl`, `ocean_*.hlsl`, `gpuculling.hlsl`.
  - **Почему эталон:** Один из самых передовых независимых C++20 движков с поддержкой DX12/Vulkan.
  - **Что брать для плана:**
    - Двухфазный GPU Occlusion Culling с генерацией Hi-Z пирамиды;
    - Генерацию и отсечение Meshlets;
    - GPU-driven симуляцию частиц (Compute + Indirect draw).
- **[Meshoptimizer (Arseny Kapoulkine)](https://github.com/zeux/meshoptimizer)**
  - **Что смотреть:** `src/clusterizer.cpp`, `src/quantizer.cpp`.
  - **Почему эталон:** Библиотека, используемая в Epic Games, Roblox и Godot. Содержит готовые алгоритмы разбиения на мешлеты, упрощения геометрии с `LockBorder` и **квантования вершин** (Octahedral snorm, 16-bit float).

---

### 4. GPU Clustered Lighting (Вычисление света на Compute)

- **[HybridRenderer (Angel Ortiz)](https://github.com/Angelo1211/HybridRenderer)**
  - **Что смотреть:** `src/shaders/clustered_forward/cluster_cull_lights.comp` и `cluster_grid_build.comp`.
  - **Почему эталон:** Каноническая открытая реализация статьи _Practical Clustered Shading (Olsson et al.)_.
  - **Что брать для плана:** Полный шейдер построения AABB кластеров $16\times 16\times 24$ и шейдер назначения источников света через Workgroup Shared Memory (`shared uint visibleLightCount`).

---

### 5. Справочники на Rust: GPU Пайплайны и ECS Граф систем

- **[Kajiya (Embark Studios)](https://github.com/EmbarkStudios/kajiya)**
  - **Язык:** Rust + Vulkan / DX12.
  - **Что смотреть:** `crates/lib/kajiya-rg` (RenderGraph) и `assets/shaders/`.
  - **Почему эталон:** Экспериментальный движок от ветеранов DICE/Frostbite на чистом Rust. Лучший пример того, как писать ультрабыстрые Compute/Render пайплайны, темпоральное накопление и связку ресурсов без оверхеда.
- **[Bevy ECS (Schedule & System Graph)](https://github.com/bevyengine/bevy/tree/main/crates/bevy_ecs)**
  - **Язык:** Rust.
  - **Что смотреть:** `src/schedule/` (построение DAG зависимостей систем).
  - **Почему эталон:** Доказанный продакшн-паттерн автоматического параллельного запуска систем на базе декларации `&T` / `&mut T` через `rayon`/потоки.

---

### 6. SDF Contact Shadows и Субпиксельные тени

- **[TheRealMJP / Shadows](https://github.com/TheRealMJP/Shadows)**
  - **Что смотреть:** `Shadows/SampleFramework11/v1.01/Shaders/` (CSM, PCSS, VSM, EVSM).
  - **Почему эталон:** Классический эталон правильной работы с субпиксельным снаппингом матриц света (`Texel Snapping`), устраняющим плавание краев CSM, и фильтрацией мягких теней.

---
