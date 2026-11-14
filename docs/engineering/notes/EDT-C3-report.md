# EDT-C3 — отчёт исполнителя

- **Статус:** ✅ (живой GPU bench — координатору)
- **Ветка:** `edt/b6-c3`

## Шаги

| Шаг | Статус |
|-----|--------|
| 1 CPU dispatcher spans | ✅ default off, tests |
| 2 GPU pass model | ✅ `duration_ms: None` / n/a |
| 3 Profiler UI + JSON export | ✅ |
| 4 `--bench` CLI | ✅ dummy timings in unit test |
| 5 Docs / limits | ✅ `BENCHMARKS.md`, `KNOWN_LIMITS.md`, `CHANGELOG` |

## Координатору проверить

```bash
cargo clippy -D warnings -p redox_core -p redox_editor_debugger -p redox_editor_ui -p redox_editor
cargo test -p redox_core system_timing
cargo test -p redox_editor_debugger
redox_editor --bench sample_project --frames 60 --out /tmp/bench.json
```

Живой GPU: pass list с ms только при `PROFILER_FEATURES`; иначе n/a.

## Закрытие §23.9

CPU spans ✅ · GPU panel честный ✅ · export ✅ · bench JSON ✅