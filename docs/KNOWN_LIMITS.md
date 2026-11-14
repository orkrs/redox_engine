# Known limits (editor / profiler)

## GPU pass timings in the Profiler tab

When the wgpu backend does not expose timestamp queries, GPU pass rows show **n/a** for duration. The editor does not estimate missing milliseconds.

Enable `TIMESTAMP_QUERY` on the device (see `redox_render::profiler::PROFILER_FEATURES`) for real GPU spans.

## CPU system spans

Dispatcher timing is **off by default** (`Dispatcher::set_system_timing_enabled`). Turn it on in the editor host when profiling; shipping game builds should leave it disabled to avoid overhead.