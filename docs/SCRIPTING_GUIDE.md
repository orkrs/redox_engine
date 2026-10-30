# Руководство по скриптам на Rust (`redox_script_sdk`)

Скрипт — это WebAssembly-модуль, который экспортирует `on_update(dt)`. Крейт `redox_script_sdk`
прячет сырые импорты `env` за функциями, которые принимают `&str` и срезы.

## Минимальный скрипт

```rust
#![cfg_attr(target_arch = "wasm32", no_std)]

fn update(_dt: f32) {
    let px = redox_script_sdk::input("px");
    if px > 2.0 {
        redox_script_sdk::emit("past_gate", px);
    }
}

redox_script_sdk::script!(update);
```

Сборка: `cargo build -p <твой_крейт> --target wasm32-unknown-unknown --release`. Крейт скрипта —
`crate-type = ["cdylib"]`. В `no_std`-режиме нужен `#[panic_handler]` (см. `examples/coins.rs`).

## Функции

| Функция | Права (`ApiPermissions`) | Что делает |
|---|---|---|
| `input(name) -> f32` | `state` | читает вход, который хост выставил перед вызовом; неизвестное имя даёт `0.0` |
| `output(name, value)` | `state` | пишет выход, который хост читает после вызова |
| `emit(name, value)` | `events` | событие; до 256 за вызов |
| `command(name, a, b, c)` | `events` | команда игре; хост применяет их после вызова по порядку |
| `system_command(op, a, b, c)` | `commands_system` | команда `sys.<op>`, например `sys.spawn.<префаб>` |
| `raycast(o, d, max) -> Option<f32>` | `world` | расстояние до поверхности или `None` |
| `raycast_entity(o, d, max) -> Option<(EntityId, f32)>` | `world` | первая попавшая сущность |
| `find_by_name(name) -> Option<EntityId>` | `query` | сущность с именем |
| `find_by_tag(tag, &mut [EntityId]) -> usize` | `query` | сущности с тегом, сколько записано |
| `position_of(id) -> Option<[f32; 3]>` | `query` | позиция сущности |
| `overlap_sphere(c, r, &mut [EntityId]) -> usize` | `query` | сущности внутри сферы |
| `overlap_box(c, half, &mut [EntityId]) -> usize` | `query` | сущности внутри бокса |
| `get_value_of(id, key) -> Option<f32>` | `entity_values` | именованное значение чужой сущности |
| `nav_path(from, to, &mut [[f32; 3]]) -> usize` | `nav` | путь по навсетке; 0 — пути нет |

Права по умолчанию в `ScriptConfig::default()` включены все. Функция, право на которую выключено,
не линкуется: вызов даёт ошибку линковки, а не молчаливый ноль.

## Лимиты хоста

- Имя (ключ, событие, команда, тег, имя сущности) — до 64 байт; длиннее — игнорируется.
- Событий и команд — до 256 за вызов; запросов к миру — до 64 за вызов (общий лимит).
- `overlap_*` — до 64 результатов; `nav_path` — до 32 точек.
- Сущности видны скрипту как `EntityId` (`u32`), это не ECS-дескриптор.
- Выключенные сущности (`sys.set_active.<id>` с 0) не находятся по имени и тегу, лучи и
  оверлэпы их пропускают.

## Пример

`crates/redox_script_sdk/examples/coins.rs` — подбор монет: читает `px`, `pz`, `radius`, по
одному разу испускает `coin` и команду `sys.spawn.sparkle`. Тест хоста:
`crates/redox_script/tests/sdk_example.rs`.

## Чего нет

Многопоточности, прямого доступа к памяти хоста и отражения компонентов целиком: скрипт работает
только с именованными полями и командами.
