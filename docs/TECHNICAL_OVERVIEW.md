# RedOx Engine — техническое описание

Модульный игровой движок на Rust, организованный как Cargo workspace. Ниже — архитектура, назначение крейтов и текущее состояние для контекста дальнейшей разработки.

---

## 1. Общая архитектура

### 1.1 Граф зависимостей

```
                    redox_math (база: векторы, матрицы, Transform)
                           │
         ┌─────────────────┼─────────────────┐
         ▼                 ▼                 ▼
    redox_ecs          (другие)         redox_render
         │                 │                 ▲
         │    redox_input  │    redox_ui ────┘
         │    redox_asset  │
         │    redox_audio  │
         ▼                 ▼
    redox_physics      redox_core
         │                 │
         └────────┬────────┘
                  ▼
            (examples: full_demo, falling_balls, …)
```

- **redox_math** — единственный крейт без внутренних зависимостей движка; от него зависят ECS и остальные подсистемы.
- **redox_ecs** — зависит только от `redox_math`; все остальные подсистемы (render, input, asset, audio, ui, physics) зависят от **redox_math** и **redox_ecs**.
- **redox_core** — объединяет render, input, asset, audio, ui (redox_physics только в dev-dependencies для примеров).
- **redox_physics** — зависит от math, ecs и core (для времени/окна и т.п.).

### 1.2 Единый Transform

Во всём движке используется один тип трансформации из **redox_math**:

```rust
// redox_math::Transform
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}
```

- Рендер берёт `Transform` и строит model matrix через `transform.matrix()`.
- Физика синхронизирует позицию/вращение с rapier через этот же `Transform`.
- Камера, объекты, источники света и т.д. описываются через компоненты ECS и общий `Transform`.

### 1.3 Связь через ECS

- **World** хранит сущности, архетипы и ресурсы.
- Подсистемы регистрируют ресурсы (`RenderContext` не в мире; в мире — `Time`, `InputState`, `PhysicsContext`, `AssetManager` и т.д.) и компоненты.
- Рендер собирает данные из мира (например, `extract_render_objects(world, render_context)`) и отдаёт их в `RenderContext` для отрисовки.

---

## 2. Крейты

### 2.1 redox_math

- **Назначение:** математическое ядро (линейная алгебра, геометрия).
- **Зависимости:** только `glam`.
- **Содержимое:**
  - Типы: `Vec2`, `Vec3`, `Vec4`, матрицы, `Quat` (реэкспорт из glam).
  - Модули: `vector`, `matrix`, `quat`, `bounds`, `frustum`, `conversions`, **`transform`**.
  - `Transform` — translation, rotation, scale; метод `matrix()` для модели.
  - AABB, Sphere, frustum culling (bounds, frustum).
- **Точка входа:** `src/math.rs` (root lib).

### 2.2 redox_ecs

- **Назначение:** архетипная ECS с запросами, событиями и ресурсами.
- **Зависимости:** `redox_math`, glam, rayon, smallvec, crossbeam-queue, hashbrown, parking_lot, log.
- **Основные типы:**
  - **Entity** — id + generation; **EntityAllocator** — пул ID с очередью свободных.
  - **World** — spawn/despawn, add/remove component, ресурсы (`insert_resource`, `get_resource`, `get_resource_mut`, `remove_resource`), доступ по сущности (`get_component`, `get_component_mut`).
  - **Archetype** — группа сущностей с одним набором компонентов; **Table** + **Edges** для миграции между архетипами.
  - **Component** — трейт `'static + Send + Sync`; blanket impl для подходящих типов.
  - **Events&lt;T&gt;** — двойная буферизация, `send`, `update` раз в кадр; **EventReader**.
  - **Parent**, **Children** — иерархия сущностей (SmallVec для списка детей).
- **Запросы:** модуль `query` (iter, par_iter, filter) для обхода по архетипам.
- **Системы:** в `system.rs` заданы дескрипторы (System, SystemStage); фактическое выполнение — в **redox_core** через **Dispatcher** и этапы (Stage).
- **Точка входа:** `src/ecs.rs`.

### 2.3 redox_physics

- **Назначение:** физика на rapier3d с привязкой к ECS.
- **Зависимости:** redox_math, redox_ecs, redox_core, rapier3d, log, crossbeam-queue, thiserror.
- **Основное:**
  - **PhysicsContext** — владеет RigidBodySet, ColliderSet, пайплайнами rapier; хранит маппинг Entity ↔ (RigidBodyHandle, ColliderHandle); очередь raycast-запросов.
  - Компоненты: **RigidBody**, **Collider**, **Velocity** (linvel, angvel), **Kinematic** (маркер: тело управляется из ECS по Transform).
  - **sync::sync_to_physics(world)** — из ECS в rapier (Transform для kinematic, Velocity для остальных).
  - **sync::sync_from_physics(world)** — из rapier в ECS (обновление Transform для не-kinematic).
  - **sync::step_physics(world, dt)** — шаг симуляции.
  - **raycast** — RaycastRequest / RaycastResult; контекст обрабатывает очередь (результаты пока не пробрасываются в ECS событиями).
- **Модули:** components, context, physics, raycast, sync, utils (конвертация vec3/quat ↔ rapier).

### 2.4 redox_render

- **Назначение:** рендеринг на wgpu (HDR, PBR, тени, IBL, заготовки SSAO/normal).
- **Зависимости:** redox_math, redox_ecs, wgpu, winit, bytemuck, image, tobj, rand, pollster.
- **Структура:**
  - **context::RenderContext** — device, queue, surface, конфиг; все пассы и буферы; хранилище мешей/материалов/текстур (пока MVP, планируется перенос в asset).
  - **Пассы:** forward (ForwardPass), **pbr** (PbrPass), **shadow** (ShadowPass), **normal** (NormalPass), **post** (tone_mapping, **ssao**).
  - **Камера:** Camera, CameraUniform, ActiveCamera.
  - **Свет:** DirectionalLight, LightUniform; тень через shadow map (SHADOW_SIZE, SHADOW_FORMAT).
  - **Материал:** Material (base_color, texture / normal / MR индексы, metallic, roughness, emissive), MaterialUniform, PBR bind groups.
  - **Меш:** Mesh, Vertex, загрузка (loader), примитивы (primitive: cube, sphere и т.д.).
  - **Ресурсы:** texture, buffer, **ibl** (IBLProcessor, IBLResource из equirect).
- **Интеграция с ECS:**
  - Компоненты: **Transform** (redox_math), **MeshHandle**, **MaterialHandle** (индексы в RenderContext).
  - **systems::extract_render_objects(world, render_context)** — собирает список **RenderObject** (model_matrix, mesh_index, material_index, color) для отрисовки.
- **Рендер кадра:** shadow pass → forward (в HDR-текстуру) → tone mapping на поверхность. SSAO и normal pass объявлены в контексте, но инициализация в `RenderContext::new` не завершена (см. текущее состояние).

### 2.5 redox_audio

- **Назначение:** звук на kira с 3D-позиционированием.
- **Зависимости:** redox_math, redox_ecs, kira, log.
- **Модули:** context (AudioContext), components (AudioEmitter, AudioListener), spatial.
- Ресурс и компоненты привязаны к ECS; позиции/направления — через redox_math (Vec3 и т.д.).

### 2.6 redox_input

- **Назначение:** ввод (клавиатура, мышь) и action mapping.
- **Зависимости:** redox_math, redox_ecs, winit, log.
- **Типы:**
  - **InputState** — ресурс ECS: KeyboardState, MouseState, ActionMap; `begin_frame()`, `action(name)`, `action_active`, `action_value`, `process_window_event`.
  - **ActionMap**, **ActionBinding**, **ActionKind** (Digital/Analog).
  - **KeyboardState**, **MouseState** (position, buttons, scroll).
- События winit обрабатываются в приложении и передаются в `InputState::process_window_event`.

### 2.7 redox_asset

- **Назначение:** асинхронная загрузка и кэширование ассетов, ручки (handles).
- **Зависимости:** redox_math, redox_ecs, image, gltf, log, crossbeam-queue.
- **Типы:**
  - **AssetId**, **Handle&lt;T&gt;** (reference-counted), **AssetStatus**.
  - **AssetStorage&lt;T&gt;** — хранилище по типу.
  - **AssetManager** — ECS-ресурс: base_path, storages по TypeId, `storage::<T>()`, `insert`, `get`, `load_async` и т.д.
- **loader** — загрузчики (например, image_loader); в перспективе интеграция с RenderContext и физикой.

### 2.8 redox_ui

- **Назначение:** UI на egui (отладка, инспектор, статистика).
- **Зависимости:** redox_math, redox_ecs, redox_render, egui, egui-wgpu, egui-winit, wgpu, winit, log.
- **Модули:** context (UiContext), debug (stats, inspector).
- Рендер через egui-wgpu; окно и input от winit.

### 2.9 redox_core

- **Назначение:** ядро приложения — окно, время, диспетчер систем.
- **Зависимости:** redox_math, redox_ecs, redox_render, redox_input, redox_asset, redox_audio, redox_ui, winit, log, env_logger, pollster, bytemuck, wgpu.
- **dev-dependencies:** redox_physics (для примеров).
- **Модули:**
  - **app::AppBuilder** — `new(config)`, `add_system(stage, fn)`, `run()` — создаёт окно, блокирующе инициализирует RenderContext, создаёт World и Time, запускает event loop; каждый кадр вызывает `dispatcher.run(world, render_context, time)`.
  - **dispatcher::Dispatcher&lt;Context&gt;** — системы по этапам; **Stage**: Input → Update → PhysicsSync (fixed step) → PostUpdate → RenderPrep → Render.
  - **time::Time** — delta, fixed timestep, `should_step_fixed()`, `consume_fixed_step()`.
  - **window** — создание окна (winit).
  - **config::EngineConfig** — настройки запуска.
- В текущей реализации контекст в диспетчере — **RenderContext**; физика и UI в примерах подключаются вручную (например, full_demo без AppBuilder).

---

## 3. Этапы выполнения (Dispatcher)

Порядок этапов в `Dispatcher::run`:

1. **Input** — чтение ввода (обновление InputState из событий делается снаружи, до run, или в системе Input).
2. **Update** — игровая логика (движение, реакция на ввод).
3. **PhysicsSync** — фиксированный шаг: несколько раз вызываются системы физики (sync_to_physics → step → sync_from_physics), пока `time.should_step_fixed()`.
4. **PostUpdate** — логика после физики.
5. **RenderPrep** — сбор данных для рендера (например, `extract_render_objects`), запись в буферы/списки.
6. **Render** — вызов `render_context.render_frame(objects)` и т.п.

Физика в примерах (falling_balls, full_demo) может подключаться как системы на PhysicsSync и Update/PostUpdate; PhysicsContext хранится в World как ресурс.

---

## 4. Текущее состояние и известные проблемы

### Работает

- Базовая графика: wgpu, forward/PBR pass, тени, HDR, tone mapping, IBL (equirect → irradiance/prefiltered/BRDF LUT).
- Физика: rapier3d, синхронизация Transform и Velocity с ECS (sync_to_physics / sync_from_physics), kinematic тела, raycast (обработка очереди в контексте).
- Ввод: клавиатура, мышь, action map; InputState как ресурс.
- UI: egui, UiContext, отладочные виджеты (FPS, инспектор).
- ECS: архетипы, компоненты, ресурсы, события, иерархия (Parent/Children).
- Примеры: full_demo (рендер, камера, свет, пол, бокс, сфера, ввод, UI), falling_balls (с физикой), simple_game, crystal_forest.

### В разработке / незавершено

- **SSAO:** модуль `post::ssao` (SSAOPass с ядром, шумом, blur) есть, но:
  - в `RenderContext::new` не инициализированы поля `normal_pass`, `ssao_pass`, `normal_texture`, `normal_view`, `ssao_raw_texture`, `ssao_raw_view`, `ssao_blurred_texture`, `ssao_blurred_view`, `ssao_bind_group`, `ssao_blur_bind_group`;
  - в `ssao.rs` используется `bytemuck::cast_slice` для `Vec4` (glam); у `glam::Vec4` может не быть Pod — нужен repr(C)-тип или безопасное копирование в буфер.
- **Normal pass:** NormalPass объявлен в контексте и импортирован, но не создаётся и не присваивается в `RenderContext::new`.
- В результате **текущая сборка падает**: E0063 (missing fields в RenderContext) и E0277 (Pod для Vec4 в SSAO).

### Рекомендации для продолжения

1. **Сборка:** в `RenderContext::new` добавить создание и присвоение всех полей (NormalPass, SSAOPass, текстуры и bind groups для SSAO/normal); при необходимости добавить resize для SSAO/normal-текстур.
2. **SSAO:** заменить запись ядра через `cast_slice` на тип с `#[repr(C)]` и Pod/Zeroable или побайтовую запись (например, `[f32; 4]` или bytemuck-совместимый тип).
3. **Интеграция SSAO в кадр:** после forward pass рендерить SSAO из depth/normal, затем blur, затем использовать в PBR (или в отдельном композитном пассе).
4. **Физика в core:** при желании добавить redox_physics в основные зависимости redox_core и встроить физику в AppBuilder (ресурс PhysicsContext, системы sync/step по умолчанию).
5. **Рендер и ассеты:** постепенно переносить меши/текстуры из RenderContext в AssetManager и связывать загрузку с рендером через Handle.

---

## 5. Файловая структура (кратко)

- **crates/redox_math/src/** — math.rs (root), vector, matrix, quat, bounds, frustum, conversions, transform.
- **crates/redox_ecs/src/** — ecs.rs (root), entity, component, archetype (column, edges, table), world, query (iter, par_iter, filter), event, system, hierarchy.
- **crates/redox_physics/src/** — lib, components, context, physics, raycast, sync, utils.
- **crates/redox_render/src/** — lib, context, camera, light, material, mesh (loader, primitive), pass (forward, normal, pbr, shadow), post (ssao, tone_mapping), resource (buffer, texture, ibl), shader, systems.
- **crates/redox_core/src/** — lib, app, config, dispatcher, time, window.
- **crates/redox_input/src/** — lib, keyboard, mouse, action, state.
- **crates/redox_asset/src/** — lib, handle, storage, manager, loader (image_loader и т.д.).
- **crates/redox_audio/src/** — lib, context, components, spatial.
- **crates/redox_ui/src/** — lib, context, debug (inspector, stats).

Workspace: `Cargo.toml` в корне задаёт members и workspace.dependencies (glam, winit, wgpu, rapier3d, kira, rayon, log, env_logger, smallvec, crossbeam-queue, image, gltf, bytemuck, anyhow, egui, egui-wgpu, egui-winit).

---

Этот документ можно использовать как базу для планирования задач по SSAO, нормальному пассу, интеграции физики в core и переносу ассетов в менеджер.
