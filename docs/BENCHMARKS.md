# RedOx editor benchmarks

## CLI

```bash
redox_editor --bench <project_dir> --frames N --out report.json
```

Optional HTML stub when `--out` ends with `.html`.

## Fields

See `redox_editor_debugger::BenchReport`: `meta.gpu_name`, `meta.driver`, `meta.git_commit`, `meta.timestamp`, `samples[]`, `avg_fps`.

Live GPU/driver metadata is filled by the coordinator when running on hardware; unit tests use dummy timings and `n/a` for GPU fields.

## Coordinator

Run a full benchmark on a machine with wgpu timestamps enabled and attach the JSON to perf regression tracking.