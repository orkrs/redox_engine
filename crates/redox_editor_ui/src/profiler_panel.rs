use redox_editor_debugger::{
    CpuTimeline, FrameProfiler, GpuPassEntry, export_profiler_frame_json, merge_gpu_timings,
    passes_from_frame_graph_report,
};
use redox_render::frame_graph::frame_graph_report;
use redox_render::profiler::GpuTiming;

/// Panel that plots frame times, CPU bars and GPU pass list.
pub struct ProfilerPanel;

impl ProfilerPanel {
    /// Draws the frame time history of `profiler` plus optional CPU/GPU data.
    pub fn show(
        ui: &mut egui::Ui,
        profiler: &FrameProfiler,
        cpu: &CpuTimeline,
        gpu: &[GpuPassEntry],
    ) {
        ui.heading("⏱ Frame Profiler");
        ui.label(format!(
            "FPS {:.0} · avg {:.2} ms · 1% low {:.0} · 0.1% low {:.0}",
            profiler.fps(),
            profiler.avg_ms(),
            profiler.one_percent_low_fps(),
            profiler.point_low_fps()
        ));
        if ui.button("Export frame JSON").clicked() {
            if let Ok(json) = export_profiler_frame_json(profiler, cpu, gpu) {
                let _ = std::fs::write("profiler_frame.json", json);
            }
        }
        ui.separator();
        ui.collapsing("CPU systems", |ui| {
            if cpu.spans.is_empty() {
                ui.label("Enable dispatcher system timing (editor flag).");
                return;
            }
            let max_ms = cpu
                .spans
                .iter()
                .map(|s| s.ms)
                .fold(0.1_f32, f32::max);
            for span in &cpu.spans {
                ui.horizontal(|ui| {
                    ui.label(&span.name);
                    let frac = (span.ms / max_ms).clamp(0.0, 1.0);
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(200.0, 14.0), egui::Sense::hover());
                    let fill = egui::Rect::from_min_size(
                        rect.min,
                        egui::vec2(rect.width() * frac, rect.height()),
                    );
                    ui.painter()
                        .rect_filled(fill, 0.0, egui::Color32::from_rgb(90, 160, 220));
                    ui.label(format!("{:.2} ms", span.ms));
                });
            }
        });
        ui.collapsing("GPU passes", |ui| {
            if gpu.is_empty() {
                ui.label("n/a");
                return;
            }
            for pass in gpu {
                let label = match pass.duration_ms {
                    Some(ms) => format!("{:.2} ms", ms),
                    None => "n/a".to_string(),
                };
                ui.label(format!("{} — {}", pass.name, label));
            }
        });
        ui.separator();
        let samples: Vec<f32> = profiler.samples_ms.iter().copied().collect();
        if samples.is_empty() {
            ui.label("Collecting samples…");
            return;
        }
        let max_ms = samples.iter().copied().fold(1.0_f32, f32::max).max(8.0);
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 120.0),
            egui::Sense::hover(),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 2.0, egui::Color32::from_gray(28));
        let n = samples.len().max(1) as f32;
        let stroke = egui::Stroke::new(1.2, egui::Color32::from_rgb(80, 200, 140));
        for i in 1..samples.len() {
            let x0 = rect.left() + (i - 1) as f32 / (n - 1.0).max(1.0) * rect.width();
            let x1 = rect.left() + i as f32 / (n - 1.0).max(1.0) * rect.width();
            let y0 = rect.bottom() - (samples[i - 1] / max_ms) * rect.height();
            let y1 = rect.bottom() - (samples[i] / max_ms) * rect.height();
            painter.line_segment([egui::pos2(x0, y0), egui::pos2(x1, y1)], stroke);
        }
        let y16 = rect.bottom() - (16.6 / max_ms) * rect.height();
        if y16 >= rect.top() && y16 <= rect.bottom() {
            painter.line_segment(
                [egui::pos2(rect.left(), y16), egui::pos2(rect.right(), y16)],
                egui::Stroke::new(1.0, egui::Color32::from_rgb(180, 80, 80)),
            );
            painter.text(
                egui::pos2(rect.left() + 4.0, y16 - 12.0),
                egui::Align2::LEFT_BOTTOM,
                "16.6ms",
                egui::FontId::monospace(10.0),
                egui::Color32::from_rgb(180, 100, 100),
            );
        }
        ui.label(format!("{} samples · red = 60 FPS budget", samples.len()));
    }

    /// Default GPU pass list from the compiled frame graph (durations n/a until GPU timings arrive).
    pub fn default_gpu_passes(width: u32, height: u32, timings: &[GpuTiming]) -> Vec<GpuPassEntry> {
        let mut entries = passes_from_frame_graph_report(&frame_graph_report(width, height));
        merge_gpu_timings(&mut entries, timings);
        entries
    }
}