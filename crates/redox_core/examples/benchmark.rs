//! Benchmark example: runs a test scene with progressive load for a fixed duration,
//! collects per-frame and per-pass timings, memory usage, and draw-call counts,
//! then prints a detailed report and optionally exports raw data.
//!
//! Run:
//!   cargo run -p redox_core --example benchmark -- [OPTIONS]
//!
//! Options:
//!   --duration N     Total benchmark duration in seconds (default: 30)
//!   --output FILE    Write JSON summary to FILE
//!   --csv FILE       Write per-frame time series to CSV for plotting
//!   --verbose        Enable debug logging
//!
//! No user input required; the process exits automatically after the benchmark.

use std::sync::Arc;
use std::time::Instant;

use winit::{
    event::{Event, WindowEvent},
    event_loop::EventLoop,
    window::WindowBuilder,
};

use redox_ecs::world::World;
use redox_math::{Mat4, Quat, Transform, Vec3};

use redox_render::camera::Camera;
use redox_render::context::RenderContext;
use redox_render::light::{DirectionalLight, LightUniform, PointLight};
use redox_render::mesh::primitive::{create_cube, create_sphere};
use redox_render::systems::{
    extract_render_objects, sync_assets_to_render, MaterialHandle, MeshHandle,
};
use redox_render::{asset_types::MaterialData, asset_types::MeshData};
use redox_asset::{AssetManager, Handle};

use rand::Rng;
use redox_ui::context::UiContext;

/// Samples current process RSS (physical memory) in bytes, if the platform supports it.
#[inline]
fn sample_memory_rss() -> Option<u64> {
    memory_stats::memory_stats().map(|m| m.physical_mem as u64)
}

// -----------------------------------------------------------------------------
// Configurable constants (phases, load)
// -----------------------------------------------------------------------------

/// Total benchmark duration in seconds.
const DEFAULT_BENCH_DURATION_SECS: u64 = 30;
/// Duration of each load phase (every N seconds we add more objects/lights).
const PHASE_DURATION_SECS: u64 = 5;
/// Initial number of procedural objects (cubes/spheres) on the floor.
const BASE_OBJECT_COUNT: usize = 80;
/// Additional objects spawned at each phase.
const OBJECTS_PER_PHASE: usize = 40;
/// Initial number of point lights (excluding directional).
const BASE_POINT_LIGHTS: usize = 2;
/// Additional point lights per phase (capped by shader limit 128).
const POINT_LIGHTS_PER_PHASE: usize = 4;

/// Floor size (half-extents for a big flat cube).
const FLOOR_SCALE: (f32, f32, f32) = (25.0, 0.5, 25.0);
/// Area for spawning objects (x/z range).
const SPAWN_RANGE: f32 = 18.0;

// -----------------------------------------------------------------------------
// Statistics collection
// -----------------------------------------------------------------------------

/// Collects raw per-frame timings (in seconds), draw-call counts, and memory samples.
#[derive(Default)]
struct FrameStats {
    /// Total frame time (event start to present).
    frame_time_secs: Vec<f32>,
    /// Asset manager update.
    asset_update_secs: Vec<f32>,
    /// Camera buffer update.
    camera_update_secs: Vec<f32>,
    /// Light uniform update.
    light_update_secs: Vec<f32>,
    /// extract_render_objects + update_model_buffer (ECS extract).
    extract_secs: Vec<f32>,
    /// Shadow pass recording (CPU side).
    shadow_pass_secs: Vec<f32>,
    /// Normal pass recording.
    normal_pass_secs: Vec<f32>,
    /// SSAO pass recording.
    ssao_pass_secs: Vec<f32>,
    /// SSAO blur pass recording.
    blur_pass_secs: Vec<f32>,
    /// Scene (PBR) pass recording.
    scene_pass_secs: Vec<f32>,
    /// Tone mapping pass recording.
    tone_mapping_secs: Vec<f32>,
    /// UI (egui) recording.
    ui_secs: Vec<f32>,
    /// Event loop / window events handling (estimated; 0 if not measured).
    events_secs: Vec<f32>,
    /// ECS systems update (e.g. dispatcher); 0 when not used.
    ecs_secs: Vec<f32>,
    /// Physics step; 0 when not used.
    physics_secs: Vec<f32>,
    /// Draw calls per frame (len of render_objects).
    draw_calls: Vec<u32>,
    /// RSS in bytes per frame (empty if memory_stats unavailable).
    memory_rss_bytes: Vec<u64>,
}

/// Phase snapshot: frame index when phase started, object count, light count.
struct PhaseSnapshot {
    start_frame: usize,
    object_count: usize,
    light_count: usize,
}

/// Returns percentile (0..100) from sorted data; 0 = min, 100 = max.
fn percentile_sorted(sorted: &[f32], p: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = (p / 100.0 * (sorted.len() as f32 - 1.0)).max(0.0) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Standard deviation (population) of values; 0 if len < 2.
fn std_dev(values: &[f32]) -> f32 {
    if values.len() < 2 {
        return 0.0;
    }
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    let variance = values.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / (values.len() as f32);
    variance.sqrt()
}

/// Returns (mean, min, max, p1, p99, p999, std_dev) for a slice; all 0 if empty.
fn time_stats(values: &[f32]) -> (f32, f32, f32, f32, f32, f32, f32) {
    if values.is_empty() {
        return (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    let min = *sorted.first().unwrap();
    let max = *sorted.last().unwrap();
    let p1 = percentile_sorted(&sorted, 1.0);
    let p99 = percentile_sorted(&sorted, 99.0);
    let p999 = percentile_sorted(&sorted, 99.9);
    let sd = std_dev(values);
    (mean, min, max, p1, p99, p999, sd)
}

/// Returns 1% low and 0.1% low FPS (1 / high percentile of frame time).
fn low_fps_from_frame_times(frame_times: &[f32], p99: f32, p999: f32) -> (f32, f32) {
    let mut sorted = frame_times.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let t99 = percentile_sorted(&sorted, p99);
    let t999 = percentile_sorted(&sorted, p999);
    let fps_1_low = if t99 > 0.0 { 1.0 / t99 } else { 0.0 };
    let fps_01_low = if t999 > 0.0 { 1.0 / t999 } else { 0.0 };
    (fps_1_low, fps_01_low)
}

impl FrameStats {
    fn record_frame(
        &mut self,
        frame_secs: f32,
        asset_secs: f32,
        camera_secs: f32,
        light_secs: f32,
        extract_secs: f32,
        shadow_secs: f32,
        normal_secs: f32,
        ssao_secs: f32,
        blur_secs: f32,
        scene_secs: f32,
        tone_secs: f32,
        ui_secs: f32,
        events_secs: f32,
        ecs_secs: f32,
        physics_secs: f32,
        draw_calls: u32,
        memory_rss_bytes: Option<u64>,
    ) {
        self.frame_time_secs.push(frame_secs);
        self.asset_update_secs.push(asset_secs);
        self.camera_update_secs.push(camera_secs);
        self.light_update_secs.push(light_secs);
        self.extract_secs.push(extract_secs);
        self.shadow_pass_secs.push(shadow_secs);
        self.normal_pass_secs.push(normal_secs);
        self.ssao_pass_secs.push(ssao_secs);
        self.blur_pass_secs.push(blur_secs);
        self.scene_pass_secs.push(scene_secs);
        self.tone_mapping_secs.push(tone_secs);
        self.ui_secs.push(ui_secs);
        self.events_secs.push(events_secs);
        self.ecs_secs.push(ecs_secs);
        self.physics_secs.push(physics_secs);
        self.draw_calls.push(draw_calls);
        self.memory_rss_bytes.push(memory_rss_bytes.unwrap_or(0));
    }

    /// Builds the full report as a single string (everything that would be logged).
    fn build_full_report(
        &self,
        phases: &[PhaseSnapshot],
        total_secs: f32,
        config: &BenchmarkConfig,
    ) -> String {
        let n = self.frame_time_secs.len();
        let mut out = String::new();
        let mut ln = |s: &str| {
            out.push_str(s);
            out.push('\n');
        };

        if n == 0 {
            ln("No frames recorded; no report.");
            return out;
        }

        let avg_frame = self.frame_time_secs.iter().sum::<f32>() / n as f32;
        let avg_fps = if avg_frame > 0.0 { 1.0 / avg_frame } else { 0.0 };
        let mut sorted_ft = self.frame_time_secs.clone();
        sorted_ft.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let min_ft = sorted_ft.first().copied().unwrap_or(0.0);
        let max_ft = sorted_ft.last().copied().unwrap_or(0.0);
        let min_fps = if max_ft > 0.0 { 1.0 / max_ft } else { 0.0 };
        let max_fps = if min_ft > 0.0 { 1.0 / min_ft } else { 0.0 };
        let (fps_1_low, fps_01_low) = low_fps_from_frame_times(&self.frame_time_secs, 99.0, 99.9);

        let total_render_ms = (self.shadow_pass_secs.iter().sum::<f32>()
            + self.normal_pass_secs.iter().sum::<f32>()
            + self.ssao_pass_secs.iter().sum::<f32>()
            + self.blur_pass_secs.iter().sum::<f32>()
            + self.scene_pass_secs.iter().sum::<f32>()
            + self.tone_mapping_secs.iter().sum::<f32>())
            / n as f32
            * 1000.0;

        let avg_draw_calls = self.draw_calls.iter().sum::<u32>() as f32 / n as f32;
        let (dc_min, dc_max, dc_p1, dc_p99, dc_std) = if self.draw_calls.is_empty() {
            (0u32, 0u32, 0.0_f32, 0.0_f32, 0.0_f32)
        } else {
            let dc_f: Vec<f32> = self.draw_calls.iter().map(|&u| u as f32).collect();
            let (_, mi, ma, p1, p99, _, s) = time_stats(&dc_f);
            (mi as u32, ma as u32, p1, p99, s)
        };

        let has_memory = self.memory_rss_bytes.iter().any(|&b| b > 0);
        let peak_rss_mb = if has_memory {
            self.memory_rss_bytes.iter().max().copied().unwrap_or(0) as f64 / 1_048_576.0
        } else {
            0.0
        };
        let avg_rss_mb = if has_memory {
            let sum: u64 = self.memory_rss_bytes.iter().sum();
            sum as f64 / self.memory_rss_bytes.len() as f64 / 1_048_576.0
        } else {
            0.0
        };

        ln("=== Benchmark Results ===");
        ln(&format!("Duration: {:.1} s", total_secs));
        ln(&format!("Frames: {}", n));
        ln(&format!(
            "Average FPS: {:.1}",
            avg_fps
        ));
        ln(&format!(
            "Min FPS: {:.1}, Max FPS: {:.1}, 1% Low: {:.1}, 0.1% Low: {:.1}",
            min_fps, max_fps, fps_1_low, fps_01_low
        ));
        ln("");
        ln("Breakdown (average ms per frame):");
        ln(&format!("  Shadow pass:    {:.2}", mean_ms(&self.shadow_pass_secs)));
        ln(&format!("  Normal pass:   {:.2}", mean_ms(&self.normal_pass_secs)));
        ln(&format!("  SSAO pass:     {:.2}", mean_ms(&self.ssao_pass_secs)));
        ln(&format!("  SSAO blur:     {:.2}", mean_ms(&self.blur_pass_secs)));
        ln(&format!("  Scene pass:    {:.2}", mean_ms(&self.scene_pass_secs)));
        ln(&format!("  Tone mapping:  {:.2}", mean_ms(&self.tone_mapping_secs)));
        ln(&format!("  Asset manager: {:.2}", mean_ms(&self.asset_update_secs)));
        ln(&format!("  UI:            {:.2}", mean_ms(&self.ui_secs)));
        ln(&format!("  Events:        {:.2}", mean_ms(&self.events_secs)));
        ln(&format!("  ECS:           {:.2}", mean_ms(&self.ecs_secs)));
        ln(&format!("  Physics:       {:.2}", mean_ms(&self.physics_secs)));
        ln(&format!("  Total render:  {:.2}", total_render_ms));
        ln("");
        ln(&format!("Draw calls (average per frame): {}", avg_draw_calls.round()));
        if config.verbose {
            ln(&format!(
                "  min={} max={} p1={:.1} p99={:.1} std={:.1}",
                dc_min, dc_max, dc_p1, dc_p99, dc_std
            ));
        }
        if has_memory {
            ln(&format!("Memory usage (peak RSS): {:.0} MB (avg: {:.0} MB)", peak_rss_mb, avg_rss_mb));
        } else {
            ln("Memory usage (peak RSS): N/A");
        }
        ln("");
        ln("Load progression (summary):");
        for (i, ph) in phases.iter().enumerate() {
            let end_frame = phases.get(i + 1).map(|p| p.start_frame).unwrap_or(n);
            let count = end_frame.saturating_sub(ph.start_frame);
            if count == 0 {
                continue;
            }
            let t0 = ph.start_frame as f32 * avg_frame;
            let t1 = (end_frame as f32 * avg_frame).min(total_secs);
            let phase_times = &self.frame_time_secs[ph.start_frame..end_frame];
            let phase_avg = phase_times.iter().sum::<f32>() / count as f32;
            let phase_fps = if phase_avg > 0.0 { 1.0 / phase_avg } else { 0.0 };
            let phase_min_ft = phase_times.iter().cloned().min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)).unwrap_or(0.0);
            let phase_min_fps = if phase_min_ft > 0.0 { 1.0 / phase_min_ft } else { 0.0 };
            let phase_dc: f32 = self.draw_calls[ph.start_frame..end_frame].iter().sum::<u32>() as f32 / count as f32;
            let phase_mem_mb = if self.memory_rss_bytes.len() >= end_frame {
                let slice = &self.memory_rss_bytes[ph.start_frame..end_frame];
                let non_zero: Vec<u64> = slice.iter().copied().filter(|&b| b > 0).collect();
                if non_zero.is_empty() { 0.0 } else { non_zero.iter().sum::<u64>() as f64 / non_zero.len() as f64 / 1_048_576.0 }
            } else { 0.0 };
            ln(&format!(
                "  Phase {} ({:.0}-{:.0}s): objects {}, lights {}, avg FPS {:.0}, min FPS {:.0}, avg draw calls {:.0}{}",
                i + 1, t0, t1, ph.object_count, ph.light_count, phase_fps, phase_min_fps, phase_dc,
                if phase_mem_mb > 0.0 { format!(", mem {:.0} MB", phase_mem_mb) } else { String::new() }
            ));
        }
        ln("");
        ln("---------- Per-phase detailed statistics ----------");
        for (i, ph) in phases.iter().enumerate() {
            let end_frame = phases.get(i + 1).map(|p| p.start_frame).unwrap_or(n);
            let count = end_frame.saturating_sub(ph.start_frame);
            if count == 0 {
                continue;
            }
            let t0 = ph.start_frame as f32 * avg_frame;
            let t1 = (end_frame as f32 * avg_frame).min(total_secs);
            let phase_ft = &self.frame_time_secs[ph.start_frame..end_frame];
            let (ft_mean, ft_min, ft_max, ft_p1, ft_p99, ft_p999, ft_std) = time_stats(phase_ft);
            let phase_avg_fps = if ft_mean > 0.0 { 1.0 / ft_mean } else { 0.0 };
            let phase_min_fps = if ft_max > 0.0 { 1.0 / ft_max } else { 0.0 };
            let phase_max_fps = if ft_min > 0.0 { 1.0 / ft_min } else { 0.0 };
            let phase_1_low_fps = if ft_p99 > 0.0 { 1.0 / ft_p99 } else { 0.0 };
            let phase_01_low_fps = if ft_p999 > 0.0 { 1.0 / ft_p999 } else { 0.0 };
            let phase_dc = &self.draw_calls[ph.start_frame..end_frame];
            let phase_avg_dc = phase_dc.iter().sum::<u32>() as f32 / count as f32;
            let phase_min_dc = phase_dc.iter().min().copied().unwrap_or(0);
            let phase_max_dc = phase_dc.iter().max().copied().unwrap_or(0);
            let phase_mem = if self.memory_rss_bytes.len() >= end_frame {
                let slice = &self.memory_rss_bytes[ph.start_frame..end_frame];
                let non_zero: Vec<u64> = slice.iter().copied().filter(|&b| b > 0).collect();
                if non_zero.is_empty() { (0.0_f64, 0.0_f64, 0.0_f64) } else {
                    (non_zero.iter().sum::<u64>() as f64 / non_zero.len() as f64 / 1_048_576.0,
                     *non_zero.iter().min().unwrap() as f64 / 1_048_576.0,
                     *non_zero.iter().max().unwrap() as f64 / 1_048_576.0)
                }
            } else { (0.0_f64, 0.0_f64, 0.0_f64) };

            ln(&format!("  --- Phase {} (time {:.1}-{:.1} s, {} frames) ---", i + 1, t0, t1, count));
            ln(&format!("      Load: objects={}, point_lights={}", ph.object_count, ph.light_count));
            ln(&format!(
                "      Frame time (ms): avg={:.3}, min={:.3}, max={:.3}, p1={:.3}, p99={:.3}, p99.9={:.3}, std={:.3}",
                ft_mean * 1000.0, ft_min * 1000.0, ft_max * 1000.0, ft_p1 * 1000.0, ft_p99 * 1000.0, ft_p999 * 1000.0, ft_std * 1000.0
            ));
            ln(&format!(
                "      FPS: avg={:.1}, min={:.1}, max={:.1}, 1% low={:.1}, 0.1% low={:.1}",
                phase_avg_fps, phase_min_fps, phase_max_fps, phase_1_low_fps, phase_01_low_fps
            ));
            ln(&format!(
                "      Pass times (avg ms): shadow={:.3}, normal={:.3}, ssao={:.3}, blur={:.3}, scene={:.3}, tone={:.3}, asset={:.3}, ui={:.3}",
                mean_ms(&self.shadow_pass_secs[ph.start_frame..end_frame]),
                mean_ms(&self.normal_pass_secs[ph.start_frame..end_frame]),
                mean_ms(&self.ssao_pass_secs[ph.start_frame..end_frame]),
                mean_ms(&self.blur_pass_secs[ph.start_frame..end_frame]),
                mean_ms(&self.scene_pass_secs[ph.start_frame..end_frame]),
                mean_ms(&self.tone_mapping_secs[ph.start_frame..end_frame]),
                mean_ms(&self.asset_update_secs[ph.start_frame..end_frame]),
                mean_ms(&self.ui_secs[ph.start_frame..end_frame])
            ));
            ln(&format!("      Draw calls: avg={:.1}, min={}, max={}", phase_avg_dc, phase_min_dc, phase_max_dc));
            if phase_mem.0 > 0.0 {
                ln(&format!("      Memory RSS (MB): avg={:.1}, min={:.1}, max={:.1}", phase_mem.0, phase_mem.1, phase_mem.2));
            }
        }
        ln("---------- End per-phase statistics ----------");
        if config.verbose {
            ln("");
            ln("Per-stage detailed (ms): mean, min, max, p1, p99, p99.9, std:");
            for (name, vals) in [
                ("shadow_pass", &self.shadow_pass_secs as &[f32]),
                ("normal_pass", &self.normal_pass_secs),
                ("ssao_pass", &self.ssao_pass_secs),
                ("blur_pass", &self.blur_pass_secs),
                ("scene_pass", &self.scene_pass_secs),
                ("tone_mapping", &self.tone_mapping_secs),
                ("asset_update", &self.asset_update_secs),
                ("ui", &self.ui_secs),
            ] {
                if vals.is_empty() {
                    ln(&format!("  {}: (no data)", name));
                } else {
                    let (mean, min, max, p1, p99, p999, sd) = time_stats(vals);
                    ln(&format!(
                        "  {}: mean={:.3}ms min={:.3} max={:.3} p1={:.3} p99={:.3} p99.9={:.3} std={:.3}ms",
                        name, mean * 1000.0, min * 1000.0, max * 1000.0, p1 * 1000.0, p99 * 1000.0, p999 * 1000.0, sd * 1000.0
                    ));
                }
            }
        }
        ln("=== End of report ===");
        out
    }

    /// Prints the full benchmark report to the log and writes the complete report to the output file.
    fn print_report(
        &self,
        phases: &[PhaseSnapshot],
        total_secs: f32,
        config: &BenchmarkConfig,
    ) {
        let n = self.frame_time_secs.len();
        if n == 0 {
            log::warn!("No frames recorded; skipping report.");
            return;
        }

        let full_report = self.build_full_report(phases, total_secs, config);

        if let Some(path) = config.output_path.as_deref() {
            if let Err(e) = std::fs::write(path, &full_report) {
                log::warn!("Failed to write report file {}: {}", path, e);
            } else {
                log::info!("Full report written to {} ({} bytes)", path, full_report.len());
            }
        }

        for line in full_report.lines() {
            log::info!("{}", line);
        }

        if let Some(path) = config.csv_path.as_deref() {
            if let Err(e) = write_csv(path, self, n) {
                log::warn!("Failed to write CSV {}: {}", path, e);
            } else {
                log::info!("CSV written to {}", path);
            }
        }
    }
}

fn mean_ms(secs: &[f32]) -> f32 {
    if secs.is_empty() {
        return 0.0;
    }
    secs.iter().sum::<f32>() / secs.len() as f32 * 1000.0
}

/// Writes per-frame time series to CSV for plotting.
fn write_csv(path: &str, stats: &FrameStats, n: usize) -> std::io::Result<()> {
    let mut out = String::new();
    out.push_str("frame,frame_ms,shadow_ms,normal_ms,ssao_ms,blur_ms,scene_ms,tone_ms,asset_ms,ui_ms,events_ms,ecs_ms,physics_ms,draw_calls,memory_mb\n");
    for i in 0..n {
        let frame_ms = stats.frame_time_secs.get(i).copied().unwrap_or(0.0) * 1000.0;
        let shadow_ms = stats.shadow_pass_secs.get(i).copied().unwrap_or(0.0) * 1000.0;
        let normal_ms = stats.normal_pass_secs.get(i).copied().unwrap_or(0.0) * 1000.0;
        let ssao_ms = stats.ssao_pass_secs.get(i).copied().unwrap_or(0.0) * 1000.0;
        let blur_ms = stats.blur_pass_secs.get(i).copied().unwrap_or(0.0) * 1000.0;
        let scene_ms = stats.scene_pass_secs.get(i).copied().unwrap_or(0.0) * 1000.0;
        let tone_ms = stats.tone_mapping_secs.get(i).copied().unwrap_or(0.0) * 1000.0;
        let asset_ms = stats.asset_update_secs.get(i).copied().unwrap_or(0.0) * 1000.0;
        let ui_ms = stats.ui_secs.get(i).copied().unwrap_or(0.0) * 1000.0;
        let events_ms = stats.events_secs.get(i).copied().unwrap_or(0.0) * 1000.0;
        let ecs_ms = stats.ecs_secs.get(i).copied().unwrap_or(0.0) * 1000.0;
        let physics_ms = stats.physics_secs.get(i).copied().unwrap_or(0.0) * 1000.0;
        let dc = stats.draw_calls.get(i).copied().unwrap_or(0);
        let mem_mb = stats
            .memory_rss_bytes
            .get(i)
            .copied()
            .map(|b| b as f64 / 1_048_576.0)
            .unwrap_or(0.0);
        out.push_str(&format!(
            "{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{},{:.2}\n",
            i, frame_ms, shadow_ms, normal_ms, ssao_ms, blur_ms, scene_ms, tone_ms,
            asset_ms, ui_ms, events_ms, ecs_ms, physics_ms, dc, mem_mb
        ));
    }
    std::fs::write(path, out)
}

// -----------------------------------------------------------------------------
// Benchmark state (phases, spawn lists, stats)
// -----------------------------------------------------------------------------

struct BenchmarkConfig {
    duration_secs: u64,
    phase_duration_secs: u64,
    output_path: Option<String>,
    csv_path: Option<String>,
    verbose: bool,
}

struct BenchmarkState {
    config: BenchmarkConfig,
    start_time: Instant,
    phase_start: Instant,
    current_phase: usize,
    /// Entity IDs of spawned objects (cubes/spheres).
    object_entities: Vec<redox_ecs::entity::Entity>,
    /// Entity IDs of point lights (no mesh).
    light_entities: Vec<redox_ecs::entity::Entity>,
    stats: FrameStats,
    /// For each phase: (start_frame, object_count, light_count).
    phases: Vec<PhaseSnapshot>,
}

impl BenchmarkState {
    fn new(config: BenchmarkConfig) -> Self {
        Self {
            config,
            start_time: Instant::now(),
            phase_start: Instant::now(),
            current_phase: 0,
            object_entities: Vec::new(),
            light_entities: Vec::new(),
            stats: FrameStats::default(),
            phases: Vec::new(),
        }
    }

    fn elapsed_secs(&self) -> f32 {
        self.start_time.elapsed().as_secs_f32()
    }

    fn is_finished(&self) -> bool {
        self.elapsed_secs() >= self.config.duration_secs as f32
    }

    /// Advance phase: add more objects and lights. Call once per phase transition.
    fn advance_phase(
        &mut self,
        world: &mut World,
        mesh_cube: &MeshHandle,
        mesh_sphere: &MeshHandle,
        mat_objects: &[MaterialHandle],
    ) {
        let phase_duration = self.config.phase_duration_secs as f32;
        if self.phase_start.elapsed().as_secs_f32() < phase_duration {
            return;
        }
        self.phase_start = Instant::now();
        self.phases.push(PhaseSnapshot {
            start_frame: self.stats.frame_time_secs.len(),
            object_count: self.object_entities.len(),
            light_count: self.light_entities.len(),
        });

        // Spawn OBJECTS_PER_PHASE new objects
        let mut rng = rand::thread_rng();
        for _ in 0..OBJECTS_PER_PHASE {
            let e = world.spawn();
            let x = (rng.gen_range(0.0_f32..1.0_f32) - 0.5) * 2.0 * SPAWN_RANGE;
            let z = (rng.gen_range(0.0_f32..1.0_f32) - 0.5) * 2.0 * SPAWN_RANGE;
            let y = 0.5 + rng.gen_range(0.0_f32..1.0_f32) * 0.5;
            let mesh = if rng.gen_range(0.0_f32..1.0_f32) > 0.5 {
                mesh_cube.clone()
            } else {
                mesh_sphere.clone()
            };
            let mat = &mat_objects[rng.gen_range(0..mat_objects.len())];
            world.add_component(
                e,
                Transform {
                    translation: Vec3::new(x, y, z),
                    rotation: Quat::IDENTITY,
                    scale: Vec3::new(0.4, 0.4, 0.4),
                },
            );
            world.add_component(e, mesh);
            world.add_component(e, mat.clone());
            self.object_entities.push(e);
        }

        // Spawn POINT_LIGHTS_PER_PHASE new point lights (max 128 total)
        let total_lights = self.light_entities.len() + POINT_LIGHTS_PER_PHASE;
        if total_lights <= 128 {
            for _ in 0..POINT_LIGHTS_PER_PHASE {
                let e = world.spawn();
                let x = (rng.gen_range(0.0_f32..1.0_f32) - 0.5) * 2.0 * SPAWN_RANGE * 0.5;
                let z = (rng.gen_range(0.0_f32..1.0_f32) - 0.5) * 2.0 * SPAWN_RANGE * 0.5;
                let y = 2.0 + rng.gen_range(0.0_f32..1.0_f32) * 2.0;
                let pos = Vec3::new(x, y, z);
                world.add_component(e, Transform { translation: pos, rotation: Quat::IDENTITY, scale: Vec3::ONE });
                world.add_component(
                    e,
                    PointLight::new(pos, Vec3::new(0.9, 0.85, 0.8), 3.0, 15.0),
                );
                self.light_entities.push(e);
            }
        }

        self.current_phase += 1;
        log::debug!(
            "Phase {}: objects={} lights={}",
            self.current_phase,
            self.object_entities.len(),
            self.light_entities.len()
        );
    }
}

// -----------------------------------------------------------------------------
// Scene setup
// -----------------------------------------------------------------------------

struct GameAssets {
    mesh_handles: Vec<Handle<MeshData>>,
    material_handles: Vec<Handle<MaterialData>>,
}

fn main() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();

    let mut args = std::env::args().skip(1);
    let mut duration_secs = DEFAULT_BENCH_DURATION_SECS;
    let mut output_path: Option<String> = None;
    let mut csv_path: Option<String> = None;
    let mut verbose = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--duration" => {
                if let Some(s) = args.next() {
                    duration_secs = s.parse().unwrap_or(DEFAULT_BENCH_DURATION_SECS);
                }
            }
            "--output" => {
                if let Some(s) = args.next() {
                    output_path = Some(s);
                }
            }
            "--csv" => {
                if let Some(s) = args.next() {
                    csv_path = Some(s);
                }
            }
            "--verbose" => verbose = true,
            _ => {}
        }
    }

    if verbose {
        log::set_max_level(log::LevelFilter::Debug);
    }
    log::info!(
        "Benchmark: duration={}s, phase every {}s, output={:?}, csv={:?}",
        duration_secs,
        PHASE_DURATION_SECS,
        output_path,
        csv_path
    );

    pollster::block_on(run_benchmark(BenchmarkConfig {
        duration_secs,
        phase_duration_secs: PHASE_DURATION_SECS,
        output_path,
        csv_path,
        verbose,
    }));
}

async fn run_benchmark(config: BenchmarkConfig) {
    let event_loop = EventLoop::new().unwrap();
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("RedOx Benchmark")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0))
            .build(&event_loop)
            .unwrap(),
    );

    let mut render_ctx = RenderContext::new(window.clone()).await;
    let mut ui_ctx = UiContext::new(&window, render_ctx.device(), render_ctx.surface_format());
    let mut world = World::new();

    // --- Assets ---
    let mut asset_manager = AssetManager::new(".");
    let mesh_cube = MeshHandle(asset_manager.insert(create_cube()));
    let mesh_sphere = MeshHandle(asset_manager.insert(create_sphere(0.5, 24)));

    let mat_floor = MaterialHandle(asset_manager.insert(
        MaterialData::solid(Vec3::new(0.1, 0.1, 0.12))
            .metallic(0.0)
            .roughness(0.8),
    ));
    let mat_obj: Vec<MaterialHandle> = (0..4)
        .map(|i| {
            let c = match i {
                0 => Vec3::new(0.2, 0.15, 0.18),
                1 => Vec3::new(0.15, 0.2, 0.18),
                2 => Vec3::new(0.18, 0.16, 0.22),
                _ => Vec3::new(0.14, 0.18, 0.2),
            };
            MaterialHandle(asset_manager.insert(
                MaterialData::solid(c).metallic(0.1).roughness(0.6),
            ))
        })
        .collect();

    let all_materials: Vec<Handle<MaterialData>> = std::iter::once(mat_floor.0)
        .chain(mat_obj.iter().map(|m| m.0))
        .collect();
    let game_assets = GameAssets {
        mesh_handles: vec![mesh_cube.0, mesh_sphere.0],
        material_handles: all_materials,
    };
    world.insert_resource(game_assets);

    // Optional IBL
    let hdr_path = "assets/skybox.hdr";
    if std::path::Path::new(hdr_path).exists() {
        if let Ok(hdr_bytes) = std::fs::read(hdr_path) {
            if let Ok(hdr_tex) = redox_render::resource::texture::Texture::from_hdr_bytes(
                render_ctx.device(),
                render_ctx.queue(),
                &hdr_bytes,
                "Skybox HDR",
            ) {
                render_ctx.set_environment(&hdr_tex);
            }
        }
    }

    // --- Floor ---
    let floor_entity = world.spawn();
    world.add_component(
        floor_entity,
        Transform {
            translation: Vec3::new(0.0, -0.5 * FLOOR_SCALE.1, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::new(FLOOR_SCALE.0 * 2.0, FLOOR_SCALE.1, FLOOR_SCALE.2 * 2.0),
        },
    );
    world.add_component(floor_entity, mesh_cube.clone());
    world.add_component(floor_entity, mat_floor.clone());

    // --- Directional light ---
    let light_entity = world.spawn();
    world.add_component(
        light_entity,
        DirectionalLight {
            direction: Vec3::new(0.3, -1.0, 0.2).normalize(),
            color: Vec3::new(0.4, 0.38, 0.45),
            intensity: 1.0,
        },
    );

    // --- Initial point lights ---
    let mut benchmark = BenchmarkState::new(config);
    {
        let mut rng = rand::thread_rng();
        for _ in 0..BASE_POINT_LIGHTS {
            let e = world.spawn();
            let x = (rng.gen_range(0.0_f32..1.0_f32) - 0.5) * 2.0 * 8.0;
            let z = (rng.gen_range(0.0_f32..1.0_f32) - 0.5) * 2.0 * 8.0;
            let pos = Vec3::new(x, 2.0, z);
            world.add_component(e, Transform { translation: pos, rotation: Quat::IDENTITY, scale: Vec3::ONE });
            world.add_component(e, PointLight::new(pos, Vec3::new(0.9, 0.85, 0.8), 3.0, 12.0));
            benchmark.light_entities.push(e);
        }
    }

    // --- Initial objects ---
    {
        let mut rng = rand::thread_rng();
        for _ in 0..BASE_OBJECT_COUNT {
            let e = world.spawn();
            let x = (rng.gen_range(0.0_f32..1.0_f32) - 0.5) * 2.0 * SPAWN_RANGE;
            let z = (rng.gen_range(0.0_f32..1.0_f32) - 0.5) * 2.0 * SPAWN_RANGE;
            let y = 0.5 + rng.gen_range(0.0_f32..1.0_f32) * 0.3;
            let mesh = if rng.gen_range(0.0_f32..1.0_f32) > 0.5 {
                mesh_cube.clone()
            } else {
                mesh_sphere.clone()
            };
            let mat = &mat_obj[rng.gen_range(0..mat_obj.len())];
            world.add_component(
                e,
                Transform {
                    translation: Vec3::new(x, y, z),
                    rotation: Quat::IDENTITY,
                    scale: Vec3::new(0.4, 0.4, 0.4),
                },
            );
            world.add_component(e, mesh);
            world.add_component(e, mat.clone());
            benchmark.object_entities.push(e);
        }
    }

    // --- Camera ---
    let camera_entity = world.spawn();
    let cam_pos = Vec3::new(0.0, 5.0, 22.0);
    let view_mat = Mat4::look_at_rh(cam_pos, Vec3::new(0.0, 0.0, 0.0), Vec3::Y);
    world.add_component(
        camera_entity,
        Camera {
            fov_y: 50.0_f32.to_radians(),
            near: 0.1,
            far: 500.0,
            aspect_ratio: 1280.0 / 720.0,
        },
    );
    world.add_component(
        camera_entity,
        Transform {
            translation: cam_pos,
            rotation: Quat::from_mat4(&view_mat.inverse()),
            scale: Vec3::ONE,
        },
    );

    // Record first phase
    benchmark.phases.push(PhaseSnapshot {
        start_frame: 0,
        object_count: benchmark.object_entities.len(),
        light_count: benchmark.light_entities.len(),
    });

    let mut finished = false;

    #[allow(deprecated)]
    let _ = event_loop.run(move |event, control_flow| {

        match &event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                control_flow.exit();
                return;
            }
            Event::WindowEvent { event: WindowEvent::Resized(physical_size), .. } => {
                render_ctx.resize(physical_size.width, physical_size.height);
                if let Some(cam) = world.get_component_mut::<Camera>(camera_entity) {
                    cam.aspect_ratio = physical_size.width as f32 / physical_size.height as f32;
                }
            }
            Event::AboutToWait => {
                if finished {
                    return;
                }
                benchmark.advance_phase(&mut world, &mesh_cube, &mesh_sphere, &mat_obj);
                window.request_redraw();
            }
            Event::WindowEvent { event: WindowEvent::RedrawRequested, .. } => {
                if finished {
                    return;
                }

                let frame_start = Instant::now();

                let t0 = Instant::now();
                asset_manager.update(&mut world);
                let asset_secs = t0.elapsed().as_secs_f32();

                if let Some(assets) = world.get_resource::<GameAssets>() {
                    sync_assets_to_render(
                        &mut render_ctx,
                        &asset_manager,
                        &assets.mesh_handles,
                        &[],
                        &assets.material_handles,
                    );
                }

                let t_cam = Instant::now();
                if let Some(cam) = world.get_component::<Camera>(camera_entity) {
                    let tf = world.get_component::<Transform>(camera_entity).unwrap();
                    let view = Mat4::from_quat(tf.rotation).inverse()
                        * Mat4::from_translation(-tf.translation);
                    let proj = cam.projection_matrix();
                    render_ctx.camera_uniform.view_proj = (proj * view).to_cols_array_2d();
                    render_ctx.camera_uniform.camera_pos =
                        [tf.translation.x, tf.translation.y, tf.translation.z, 1.0];
                    render_ctx.update_camera_buffer();
                }
                let camera_secs = t_cam.elapsed().as_secs_f32();

                let t_light = Instant::now();
                if let Some(light) = world.get_component::<DirectionalLight>(light_entity) {
                    let mut light_u = LightUniform {
                        dir_direction: [
                            light.direction.x, light.direction.y, light.direction.z, 0.0,
                        ],
                        dir_color: [light.color.x, light.color.y, light.color.z, light.intensity],
                        ambient: [0.08, 0.08, 0.1, 1.0],
                        ..Default::default()
                    };
                    for e in world.all_entities() {
                        if let Some(pl) = world.get_component::<PointLight>(e) {
                            let mut pl_cloned = pl.clone();
                            if let Some(tf) = world.get_component::<Transform>(e) {
                                pl_cloned.position = tf.translation;
                            }
                            light_u.add_point_light(&pl_cloned);
                        }
                    }
                    render_ctx.update_light_buffer(&light_u);
                }
                let light_secs = t_light.elapsed().as_secs_f32();

                let t_extract = Instant::now();
                let render_objects = extract_render_objects(&world, &render_ctx);
                let draw_calls = render_objects.len() as u32;
                render_ctx.update_model_buffer(&render_objects);
                let extract_secs = t_extract.elapsed().as_secs_f32();

                let output = match render_ctx.surface().get_current_texture() {
                    Ok(o) => o,
                    Err(_) => return,
                };
                let surface_view = output
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                let mut encoder = render_ctx
                    .device()
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Bench Encoder"),
                    });

                let shadow_matrix = {
                    if let Some(light) = world.get_component::<DirectionalLight>(light_entity) {
                        let size = 30.0;
                        let proj = redox_math::orthographic(-size, size, -size, size, -60.0, 60.0);
                        let view = redox_math::look_at(
                            light.direction * -25.0,
                            redox_math::Vec3::ZERO,
                            redox_math::Vec3::Y,
                        );
                        proj * view
                    } else {
                        redox_math::Mat4::IDENTITY
                    }
                };

                if let Some(light) = world.get_component::<DirectionalLight>(light_entity) {
                    let mut light_u = LightUniform {
                        dir_direction: [
                            light.direction.x, light.direction.y, light.direction.z, 0.0,
                        ],
                        dir_color: [light.color.x, light.color.y, light.color.z, light.intensity],
                        ambient: [0.08, 0.08, 0.1, 1.0],
                        shadow_view_proj: shadow_matrix.to_cols_array_2d(),
                        ..Default::default()
                    };
                    for e in world.all_entities() {
                        if let Some(pl) = world.get_component::<PointLight>(e) {
                            let mut pl_cloned = pl.clone();
                            if let Some(tf) = world.get_component::<Transform>(e) {
                                pl_cloned.position = tf.translation;
                            }
                            light_u.add_point_light(&pl_cloned);
                        }
                    }
                    render_ctx.update_light_buffer(&light_u);
                }

                let t_shadow = Instant::now();
                {
                    let shadow_view_proj_buffer =
                        redox_render::resource::buffer::create_uniform_buffer(
                            render_ctx.device(),
                            "shadow_matrix",
                            bytemuck::bytes_of(&shadow_matrix.to_cols_array_2d()),
                        );
                    let shadow_bg = render_ctx.device().create_bind_group(
                        &wgpu::BindGroupDescriptor {
                            label: Some("shadow_bg"),
                            layout: &render_ctx.shadow_pass.bind_group_layout,
                            entries: &[wgpu::BindGroupEntry {
                                binding: 0,
                                resource: shadow_view_proj_buffer.as_entire_binding(),
                            }],
                        },
                    );
                    let mut s_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("shadow_pass"),
                        color_attachments: &[],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &render_ctx.shadow_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.0),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    s_pass.set_pipeline(&render_ctx.shadow_pass.pipeline);
                    s_pass.set_bind_group(0, &shadow_bg, &[]);
                    s_pass.set_bind_group(1, &render_ctx.shadow_model_bind_group, &[]);
                    for (i, obj) in render_objects.iter().enumerate() {
                        if let Some(gpu_mesh) = render_ctx.meshes.get(obj.mesh_index) {
                            s_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                            s_pass.set_index_buffer(
                                gpu_mesh.index_buffer.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            s_pass.draw_indexed(
                                0..gpu_mesh.index_count,
                                0,
                                (i as u32)..(i as u32 + 1),
                            );
                        }
                    }
                }
                let shadow_secs = t_shadow.elapsed().as_secs_f32();

                let t_normal = Instant::now();
                {
                    let mut normal_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("normal_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &render_ctx.normal_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: 0.0, g: 0.0, b: 0.0, a: 1.0,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &render_ctx.depth_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.0),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        ..Default::default()
                    });
                    normal_pass.set_pipeline(&render_ctx.normal_pass.pipeline);
                    normal_pass.set_bind_group(0, &render_ctx.normal_bind_group, &[]);
                    for (i, obj) in render_objects.iter().enumerate() {
                        if let Some(gpu_mesh) = render_ctx.meshes.get(obj.mesh_index) {
                            normal_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                            normal_pass.set_index_buffer(
                                gpu_mesh.index_buffer.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            normal_pass.draw_indexed(
                                0..gpu_mesh.index_count,
                                0,
                                (i as u32)..(i as u32 + 1),
                            );
                        }
                    }
                }
                let normal_secs = t_normal.elapsed().as_secs_f32();

                let t_ssao = Instant::now();
                {
                    let mut ssao_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("ssao_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &render_ctx.ssao_raw_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: 1.0, g: 0.0, b: 0.0, a: 1.0,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        ..Default::default()
                    });
                    ssao_pass.set_pipeline(&render_ctx.ssao_pass.pipeline);
                    ssao_pass.set_bind_group(0, &render_ctx.ssao_bind_group, &[]);
                    ssao_pass.draw(0..3, 0..1);
                }
                let ssao_secs = t_ssao.elapsed().as_secs_f32();

                let t_blur = Instant::now();
                {
                    let mut blur_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("ssao_blur_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &render_ctx.ssao_blurred_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: 1.0, g: 0.0, b: 0.0, a: 1.0,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        ..Default::default()
                    });
                    blur_pass.set_pipeline(&render_ctx.ssao_pass.blur_pipeline);
                    blur_pass.set_bind_group(0, &render_ctx.ssao_blur_bind_group, &[]);
                    blur_pass.draw(0..3, 0..1);
                }
                let blur_secs = t_blur.elapsed().as_secs_f32();

                let t_scene = Instant::now();
                {
                    let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("scene_pass_hdr"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &render_ctx.hdr_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: 0.05, g: 0.05, b: 0.08, a: 1.0,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &render_ctx.depth_view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.0),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        ..Default::default()
                    });
                    render_ctx.record_draw(&mut rpass, &render_objects);
                }
                let scene_secs = t_scene.elapsed().as_secs_f32();

                let t_tone = Instant::now();
                {
                    let mut final_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("final_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &surface_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        ..Default::default()
                    });
                    final_pass.set_pipeline(&render_ctx.tone_mapping_pipeline);
                    final_pass.set_bind_group(0, &render_ctx.tone_mapping_bind_group, &[]);
                    final_pass.draw(0..3, 0..1);
                }
                let tone_secs = t_tone.elapsed().as_secs_f32();

                let t_ui = Instant::now();
                ui_ctx.record_frame(1.0 / 60.0);
                ui_ctx.begin_frame(&window);
                // Minimal UI for benchmark (optional: show phase/fps)
                egui::Window::new("Benchmark")
                    .anchor(egui::Align2::LEFT_TOP, [10.0, 10.0])
                    .show(&ui_ctx.egui_ctx, |ui| {
                        ui.label(format!("Phase: {} | Objects: {} | Lights: {}", benchmark.current_phase, benchmark.object_entities.len(), benchmark.light_entities.len()));
                        ui.label(format!("Frames: {} | Elapsed: {:.1}s", benchmark.stats.frame_time_secs.len(), benchmark.elapsed_secs()));
                    });
                let screen_desc = egui_wgpu::ScreenDescriptor {
                    size_in_pixels: [render_ctx.config().width, render_ctx.config().height],
                    pixels_per_point: window.scale_factor() as f32,
                };
                ui_ctx.end_frame_and_render(
                    render_ctx.device(),
                    render_ctx.queue(),
                    &mut encoder,
                    &window,
                    &surface_view,
                    screen_desc,
                );
                let ui_secs = t_ui.elapsed().as_secs_f32();

                render_ctx.queue().submit(std::iter::once(encoder.finish()));
                output.present();

                let frame_secs = frame_start.elapsed().as_secs_f32();
                let memory_rss = sample_memory_rss();
                benchmark.stats.record_frame(
                    frame_secs,
                    asset_secs,
                    camera_secs,
                    light_secs,
                    extract_secs,
                    shadow_secs,
                    normal_secs,
                    ssao_secs,
                    blur_secs,
                    scene_secs,
                    tone_secs,
                    ui_secs,
                    0.0,  // events_secs (not measured separately)
                    0.0,  // ecs_secs (no dispatcher in this benchmark)
                    0.0,  // physics_secs (physics disabled)
                    draw_calls,
                    memory_rss,
                );

                if benchmark.is_finished() {
                    finished = true;
                    let total_secs = benchmark.elapsed_secs();
                    benchmark.stats.print_report(
                        &benchmark.phases,
                        total_secs,
                        &benchmark.config,
                    );
                    log::info!("Benchmark complete. Exiting.");
                    control_flow.exit();
                }
            }
            _ => {}
        }
    });
}
