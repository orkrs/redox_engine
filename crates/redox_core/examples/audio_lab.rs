//! Sound lab (AUD-07, step 3).
//!
//! Without arguments (or with `--window`) it opens a window with four tabs: **Cue** (play a cue and
//! see which variant, volume and pitch were chosen, and whether the cooldown or the voice budget
//! stopped it), **Музыка** (move the control value of an adaptive track and see the gain of each
//! layer), **3D** (drag a source around the listener: distance and bearing) and **Анализ** (the
//! validation findings of the library and, with `--assets`, the loudness, peak and loop seam of each
//! sound file, measured once per session and again on the button).
//!
//! `--cues <cues.ron>` loads a cue library (the built-in one when absent), prints its cues and the
//! validation findings, and exits with a failure code when there is an error. `--assets <dir>` also
//! checks that each sound file is in the directory. With `--cues` and without `--window` there is
//! no window. In the window the library file is read again every second, so an edit shows up
//! without a restart; the button reloads at once.
//!
//! ASSUMPTION: a variant is found as a file named exactly like it, or with a `.wav` or `.ogg`
//! extension. The engine's asset naming was not checked against the loader.
//! `--selftest` runs the built-in check of cue selection without an output device.
//!
//! The window plays no sound files: it shows what the cue selection decides. Registering the sound
//! data and playing it through the output device is left to the game. The analysis thresholds are
//! the ones of `redox_cook audio-check` and must be kept in step with it.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use redox_audio::{
    AdaptiveMusic, AudioContext, AudioStats, CueLibrary, CueSelector, Finding, MusicLayer,
    Severity, analyze_samples, decode_bytes, loop_seam_error, validate_cue_ron,
};

/// Library used by `--selftest`. Covers a cue with variants and a cue with a cooldown.
const SELFTEST_LIBRARY: &str = r#"[
    (name: "step", variants: ["s1", "s2"], volume: (0.8, 1.0), pitch: (0.95, 1.05),
     cooldown: 0.0, bus: Sfx, priority: 1),
    (name: "hit", variants: ["h1"], volume: (1.0, 1.0), pitch: (0.9, 1.1),
     cooldown: 1.0, bus: Sfx, priority: 5),
]"#;

/// Loudness target of music, in LUFS. Same value as `redox_cook audio-check`.
const TARGET_LUFS_MUSIC: f32 = -16.0;
/// Loudness target of sound effects, in LUFS. Same value as `redox_cook audio-check`.
const TARGET_LUFS_SFX: f32 = -18.0;
/// Allowed distance from the target, in LU. Same value as `redox_cook audio-check`.
const LOUDNESS_TOLERANCE_LU: f32 = 3.0;
/// A peak above this level is clipping. Same value as `redox_cook audio-check`.
const CLIPPING_PEAK_DB: f32 = -0.1;
/// A loop seam that differs by more than this is a problem. Same value as `redox_cook audio-check`.
const SEAM_LIMIT_DB: f32 = 3.0;
/// Length of each end of a clip compared at the loop seam. Same value as `redox_cook audio-check`.
const SEAM_WINDOW_SECONDS: f32 = 0.05;
/// A sound effect longer than this is a problem. Same value as `redox_cook audio-check`.
const SFX_MAX_SECONDS: f32 = 10.0;
/// How often the window reads the cue library again.
const RELOAD_INTERVAL_SECONDS: f32 = 1.0;
/// Half the side of the 3D plane, in metres: the source stays within it.
const SPATIAL_RANGE: f32 = 20.0;

/// One sound file in the Анализ tab: its measures and the problems found in them.
#[derive(Clone, Debug, PartialEq)]
pub struct AnalysisRow {
    /// File name relative to the analysed folder.
    pub file: String,
    /// Measures, or `None` when the file could not be read or decoded.
    pub stats: Option<AudioStats>,
    /// Loop seam difference in dB, for music only.
    pub seam_db: Option<f32>,
    /// Problems found in the measures (warnings).
    pub problems: Vec<String>,
    /// Why the file could not be measured (an error).
    pub error: Option<String>,
}

/// The state of the lab for one cue library: its text, parsed cues, validation findings and the
/// analysis of the sound files.
#[derive(Debug, Default)]
pub struct LabModel {
    text: String,
    library: CueLibrary,
    findings: Vec<Finding>,
    analysis: Vec<AnalysisRow>,
}

impl LabModel {
    /// Loads `text` and validates it. `exists` tells whether a sound file is available.
    pub fn new(text: &str, exists: &dyn Fn(&str) -> bool) -> Self {
        let mut model = Self::default();
        model.reload(text, exists);
        model
    }

    /// Replaces the library when `text` differs from the loaded text.
    ///
    /// Returns true when the library changed. A text that does not parse leaves an empty library
    /// and an error finding.
    pub fn reload(&mut self, text: &str, exists: &dyn Fn(&str) -> bool) -> bool {
        if self.text == text {
            return false;
        }
        self.text = text.to_owned();
        self.library = CueLibrary::parse(text).unwrap_or_default();
        self.findings = validate_cue_ron(text, exists);
        true
    }

    /// The parsed library (empty when the text did not parse).
    pub fn library(&self) -> &CueLibrary {
        &self.library
    }

    /// Names of the cues, in the order they were defined.
    pub fn names(&self) -> Vec<String> {
        self.library.cues().map(|cue| cue.name.clone()).collect()
    }

    /// All findings of the last load.
    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }

    /// True when at least one finding is an error.
    pub fn has_errors(&self) -> bool {
        self.findings.iter().any(|f| f.severity == Severity::Error)
    }

    /// The analysis rows of the last measurement.
    pub fn analysis(&self) -> &[AnalysisRow] {
        &self.analysis
    }

    /// Replaces the analysis rows.
    pub fn set_analysis(&mut self, rows: Vec<AnalysisRow>) {
        self.analysis = rows;
    }
}

/// Problems of one measured file, by the rules of `redox_cook audio-check`.
///
/// A silent file is reported alone, because its loudness is meaningless.
pub fn problems_for(stats: &AudioStats, seam_db: Option<f32>, music: bool) -> Vec<String> {
    let mut problems = Vec::new();
    if stats.is_silent() {
        problems.push("the file is silent".to_owned());
    } else {
        let (kind, target) = if music {
            ("music", TARGET_LUFS_MUSIC)
        } else {
            ("sound", TARGET_LUFS_SFX)
        };
        if (stats.lufs_integrated - target).abs() > LOUDNESS_TOLERANCE_LU {
            problems.push(format!(
                "{kind} loudness {:.2} LUFS is outside {target} +/- {LOUDNESS_TOLERANCE_LU} LU",
                stats.lufs_integrated
            ));
        }
    }
    if stats.peak_db > CLIPPING_PEAK_DB {
        problems.push(format!("clipping: peak {:.2} dBFS", stats.peak_db));
    }
    if let Some(seam) = seam_db
        && seam > SEAM_LIMIT_DB
    {
        problems.push(format!("loop seam differs by {seam:.2} dB"));
    }
    if !music && stats.duration > SFX_MAX_SECONDS {
        problems.push(format!(
            "{:.2} s is longer than {SFX_MAX_SECONDS} s",
            stats.duration
        ));
    }
    problems
}

/// Builds the row of one file from its measurement, or from the reason it could not be measured.
pub fn analysis_row(
    file: &str,
    measured: Result<(AudioStats, Option<f32>), String>,
    music: bool,
) -> AnalysisRow {
    match measured {
        Ok((stats, seam_db)) => AnalysisRow {
            file: file.to_owned(),
            stats: Some(stats),
            seam_db,
            problems: problems_for(&stats, seam_db, music),
            error: None,
        },
        Err(error) => AnalysisRow {
            file: file.to_owned(),
            stats: None,
            seam_db: None,
            problems: Vec::new(),
            error: Some(error),
        },
    }
}

/// True when a folder of `relative` is named `music` (any case). The same marker as
/// `redox_cook audio-check`.
pub fn is_music_path(relative: &Path) -> bool {
    relative.parent().is_some_and(|dir| {
        dir.components()
            .any(|part| part.as_os_str().eq_ignore_ascii_case("music"))
    })
}

/// Distance between the listener and a source on the ground plane, in metres.
///
/// Used by the 3D tab to show the distance of the source.
fn listener_distance(listener: [f32; 2], source: [f32; 2]) -> f32 {
    (source[0] - listener[0]).hypot(source[1] - listener[1])
}

/// Bearing of `source` seen from the listener at the origin looking along +Z, in degrees
/// (0 straight ahead, positive to the right).
fn listener_bearing(source: [f32; 2]) -> f32 {
    source[0].atan2(source[1]).to_degrees()
}

/// Screen position of a point of the plane (x to the right, z up the screen) inside `rect`.
fn world_to_plane(point: [f32; 2], rect: egui::Rect) -> egui::Pos2 {
    let x = (point[0] / SPATIAL_RANGE + 1.0) * 0.5;
    let z = (point[1] / SPATIAL_RANGE + 1.0) * 0.5;
    egui::pos2(
        rect.left() + x * rect.width(),
        rect.bottom() - z * rect.height(),
    )
}

/// Point of the plane under `pos` inside `rect`, kept within the range of the plane.
fn plane_to_world(pos: egui::Pos2, rect: egui::Rect) -> [f32; 2] {
    let x = (pos.x - rect.left()) / rect.width() * 2.0 - 1.0;
    let z = (rect.bottom() - pos.y) / rect.height() * 2.0 - 1.0;
    [
        (x * SPATIAL_RANGE).clamp(-SPATIAL_RANGE, SPATIAL_RANGE),
        (z * SPATIAL_RANGE).clamp(-SPATIAL_RANGE, SPATIAL_RANGE),
    ]
}

fn layer(track: &str, fade_in_from: f32, full_from: f32) -> MusicLayer {
    MusicLayer {
        track: track.to_owned(),
        fade_in_from,
        full_from,
    }
}

/// The adaptive track "Тревога": a base layer, a mid layer and a high layer.
fn demo_music() -> AdaptiveMusic {
    AdaptiveMusic {
        layers: vec![
            layer("base", 0.0, 0.0),
            layer("mid", 0.3, 0.6),
            layer("high", 0.7, 1.0),
        ],
    }
}

/// The adaptive track "Тихо": a base layer and a pad that comes in with the control.
fn calm_music() -> AdaptiveMusic {
    AdaptiveMusic {
        layers: vec![layer("base", 0.0, 0.0), layer("pad", 0.5, 0.8)],
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Cue,
    Music,
    Spatial,
    Analysis,
}

/// What the window keeps between frames.
struct LabState {
    model: LabModel,
    audio: AudioContext,
    selector: CueSelector,
    tab: Tab,
    log: Vec<String>,
    tracks: Vec<(&'static str, AdaptiveMusic)>,
    track: usize,
    control: f32,
    source: [f32; 2],
    cues_path: Option<PathBuf>,
    assets: Option<PathBuf>,
    load_error: Option<String>,
    next_reload: f32,
    analysis_loaded: bool,
}

impl LabState {
    fn new(model: LabModel, cues_path: Option<PathBuf>, assets: Option<PathBuf>) -> Self {
        Self {
            model,
            audio: AudioContext::new(),
            selector: CueSelector::new(),
            tab: Tab::Cue,
            log: Vec::new(),
            tracks: vec![("Тревога", demo_music()), ("Тихо", calm_music())],
            track: 0,
            control: 0.0,
            source: [3.0, 4.0],
            cues_path,
            assets,
            load_error: None,
            next_reload: 0.0,
            analysis_loaded: false,
        }
    }

    fn push_log(&mut self, line: String) {
        self.log.push(line);
        if self.log.len() > 200 {
            self.log.remove(0);
        }
    }

    /// Plays `name` at `now` seconds and records what happened.
    fn play(&mut self, name: &str, now: f32) {
        let line = match self
            .audio
            .play_cue(self.model.library(), &mut self.selector, name, now)
        {
            Some(play) => format!(
                "{now:6.2}s  {name}: variant {} volume {:.2} pitch {:.2}",
                play.variant, play.volume, play.pitch
            ),
            None => format!("{now:6.2}s  {name}: not played (cooldown, no voice or unknown cue)"),
        };
        self.push_log(line);
    }

    /// Reads the cue library file again and applies it when its text changed.
    ///
    /// Returns true when the library changed. Without a library file there is nothing to read.
    fn reload_from_disk(&mut self) -> Result<bool, String> {
        let Some(path) = &self.cues_path else {
            return Ok(false);
        };
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let assets = &self.assets;
        Ok(self
            .model
            .reload(&text, &|name: &str| sound_exists(assets, name)))
    }

    /// Reloads the library now and records the outcome.
    fn reload_now(&mut self, now: f32) {
        match self.reload_from_disk() {
            Ok(true) => {
                self.load_error = None;
                self.push_log(format!("{now:6.2}s  cue library reloaded"));
            }
            Ok(false) => self.load_error = None,
            Err(error) => self.load_error = Some(error),
        }
    }

    /// Reloads the library once every `RELOAD_INTERVAL_SECONDS`.
    fn poll_library(&mut self, now: f32) {
        if now >= self.next_reload {
            self.next_reload = now + RELOAD_INTERVAL_SECONDS;
            self.reload_now(now);
        }
    }

    fn draw(&mut self, ctx: &egui::Context, now: f32) {
        self.poll_library(now);
        egui::Window::new("Audio lab")
            .anchor(egui::Align2::LEFT_TOP, [10.0, 10.0])
            .default_width(640.0)
            .show(ctx, |ui| {
                if let Some(error) = &self.load_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.tab, Tab::Cue, "Cue");
                    ui.selectable_value(&mut self.tab, Tab::Music, "Музыка");
                    ui.selectable_value(&mut self.tab, Tab::Spatial, "3D");
                    ui.selectable_value(&mut self.tab, Tab::Analysis, "Анализ");
                    if self.cues_path.is_some() && ui.button("Перезагрузить библиотеку").clicked()
                    {
                        self.reload_now(now);
                    }
                });
                ui.separator();
                match self.tab {
                    Tab::Cue => self.cue_tab(ui, now),
                    Tab::Music => self.music_tab(ui),
                    Tab::Spatial => self.spatial_tab(ui),
                    Tab::Analysis => self.analysis_tab(ui),
                }
            });
    }

    fn cue_tab(&mut self, ui: &mut egui::Ui, now: f32) {
        let mut to_play = None;
        for name in self.model.names() {
            let (priority, cooldown, variants) = match self.model.library().get(&name) {
                Some(cue) => (cue.priority, cue.cooldown, cue.variants.len()),
                None => continue,
            };
            ui.horizontal(|ui| {
                if ui.button(format!("Play {name}")).clicked() {
                    to_play = Some(name.clone());
                }
                ui.label(format!(
                    "{variants} variant(s), priority {priority}, cooldown {cooldown:.2} s"
                ));
            });
        }
        if let Some(name) = to_play {
            self.play(&name, now);
        }
        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for line in self.log.iter().rev() {
                ui.monospace(line);
            }
        });
    }

    fn music_tab(&mut self, ui: &mut egui::Ui) {
        egui::ComboBox::from_label("track")
            .selected_text(self.tracks[self.track].0)
            .show_ui(ui, |ui| {
                for (index, (name, _)) in self.tracks.iter().enumerate() {
                    ui.selectable_value(&mut self.track, index, *name);
                }
            });
        ui.add(egui::Slider::new(&mut self.control, 0.0..=1.0).text("control (tension)"));
        let (_, music) = &self.tracks[self.track];
        let gains = music.gains(self.control);
        for (layer, gain) in music.layers.iter().zip(gains) {
            ui.add(
                egui::ProgressBar::new(gain)
                    .text(format!(
                        "{} (in from {:.1}, full from {:.1}): {gain:.2}",
                        layer.track, layer.fade_in_from, layer.full_from
                    ))
                    .desired_width(420.0),
            );
        }
    }

    fn spatial_tab(&mut self, ui: &mut egui::Ui) {
        ui.label("The listener stands in the centre looking up the plane. Drag the source.");
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(300.0, 300.0), egui::Sense::click_and_drag());
        if response.dragged() || response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                self.source = plane_to_world(pos, rect);
            }
        }
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, egui::Color32::from_gray(30));
        let listener = world_to_plane([0.0, 0.0], rect);
        let source = world_to_plane(self.source, rect);
        painter.line_segment(
            [listener, source],
            egui::Stroke::new(1.0, egui::Color32::GRAY),
        );
        painter.circle_filled(listener, 6.0, egui::Color32::LIGHT_BLUE);
        painter.circle_filled(source, 8.0, egui::Color32::ORANGE);
        ui.label(format!(
            "source x {:.1} m (right), z {:.1} m (ahead)",
            self.source[0], self.source[1]
        ));
        ui.label(format!(
            "distance {:.2} m, bearing {:.0} degrees",
            listener_distance([0.0, 0.0], self.source),
            listener_bearing(self.source)
        ));
        ui.label(
            "The level falls with distance on the spatial track; walls between the source and the \
             listener muffle it (see AudioContext::update_spatial_parameters).",
        );
    }

    fn analysis_tab(&mut self, ui: &mut egui::Ui) {
        self.check_tab(ui);
        ui.separator();
        let Some(dir) = self.assets.clone() else {
            ui.label("Pass --assets <dir> to measure the sound files.");
            return;
        };
        let recount = ui.button("Пересчитать").clicked();
        if recount || !self.analysis_loaded {
            self.model.set_analysis(load_analysis(&dir));
            self.analysis_loaded = true;
        }
        if self.model.analysis().is_empty() {
            ui.label("No .wav or .ogg files in the folder.");
        }
        egui::ScrollArea::vertical()
            .max_height(260.0)
            .show(ui, |ui| {
                egui::Grid::new("analysis_grid")
                    .striped(true)
                    .show(ui, |ui| {
                        for head in ["File", "LUFS", "Peak, dBFS", "Length, s", "Seam, dB", "Problems"]
                        {
                            ui.strong(head);
                        }
                        ui.end_row();
                        for row in self.model.analysis() {
                            analysis_cells(ui, row);
                            ui.end_row();
                        }
                    });
            });
    }

    fn check_tab(&self, ui: &mut egui::Ui) {
        ui.strong("Library check");
        if self.model.findings().is_empty() {
            ui.label("No findings.");
        }
        for finding in self.model.findings() {
            let cue = finding.cue.as_deref().unwrap_or("-");
            let color = match finding.severity {
                Severity::Error => egui::Color32::LIGHT_RED,
                _ => egui::Color32::YELLOW,
            };
            ui.colored_label(
                color,
                format!("{:?} [{cue}] {}", finding.severity, finding.message),
            );
        }
    }
}

/// Colour of an analysis row: red when it could not be measured, yellow with problems.
fn row_color(row: &AnalysisRow) -> egui::Color32 {
    if row.error.is_some() {
        egui::Color32::LIGHT_RED
    } else if !row.problems.is_empty() {
        egui::Color32::YELLOW
    } else {
        egui::Color32::LIGHT_GRAY
    }
}

/// Draws the cells of one analysis row. The caller ends the row.
fn analysis_cells(ui: &mut egui::Ui, row: &AnalysisRow) {
    let color = row_color(row);
    let [lufs, peak, length] = match &row.stats {
        Some(stats) => [
            format!("{:.1}", stats.lufs_integrated),
            format!("{:.1}", stats.peak_db),
            format!("{:.2}", stats.duration),
        ],
        None => ["-".to_owned(), "-".to_owned(), "-".to_owned()],
    };
    let seam = row
        .seam_db
        .map_or_else(|| "-".to_owned(), |seam| format!("{seam:.2}"));
    let verdict = match (&row.error, row.problems.is_empty()) {
        (Some(error), _) => error.clone(),
        (None, true) => "ok".to_owned(),
        (None, false) => row.problems.join("; "),
    };
    for text in [row.file.clone(), lufs, peak, length, seam, verdict] {
        ui.label(egui::RichText::new(text).color(color));
    }
}

/// Measures one sound file: its statistics and, for music, the loop seam.
fn measure_file(path: &Path, music: bool) -> Result<(AudioStats, Option<f32>), String> {
    let bytes = std::fs::read(path).map_err(|error| format!("cannot read: {error}"))?;
    let decoded = decode_bytes(&bytes).map_err(|error| error.to_string())?;
    let stats = analyze_samples(&decoded.samples, decoded.channels, decoded.sample_rate);
    let seam = music.then(|| {
        let window = (SEAM_WINDOW_SECONDS * decoded.sample_rate as f32) as usize;
        loop_seam_error(&decoded.samples, decoded.channels, window)
    });
    Ok((stats, seam))
}

fn is_sound_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("wav") || extension.eq_ignore_ascii_case("ogg")
        })
}

/// Collects the sound files under `dir`, in any sub-folder.
fn collect_sound_files(dir: &Path, found: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_sound_files(&path, found)?;
        } else if is_sound_file(&path) {
            found.push(path);
        }
    }
    Ok(())
}

/// Measures every sound file under `dir`. A folder that cannot be read gives one error row.
fn load_analysis(dir: &Path) -> Vec<AnalysisRow> {
    let mut files = Vec::new();
    if let Err(error) = collect_sound_files(dir, &mut files) {
        let reason = format!("cannot read folder: {error}");
        return vec![analysis_row(&dir.display().to_string(), Err(reason), false)];
    }
    files.sort();
    files
        .iter()
        .map(|path| {
            let relative = path.strip_prefix(dir).unwrap_or(path);
            let music = is_music_path(relative);
            let name = relative.display().to_string();
            analysis_row(&name, measure_file(path, music), music)
        })
        .collect()
}

/// Opens the lab window and runs it until it is closed.
fn run_window(model: LabModel, cues_path: Option<PathBuf>, assets: Option<PathBuf>) -> ExitCode {
    use redox_core::config::EngineConfig;
    use redox_core::window::{RunClosure, create_window};
    use redox_math::{Quat, Vec3};
    use redox_render::camera::Camera;
    use redox_render::context::RenderContext;
    use redox_render::frame::FrameView;
    use redox_render::light::{DirectionalLight, LightScene};
    use redox_ui::UiContext;
    use winit::event::{Event, WindowEvent};

    let mut config = EngineConfig::default();
    config.window_title = "RedOx audio lab".to_string();
    config.window_width = 900;
    config.window_height = 600;
    let (event_loop, window) = create_window(&config);
    let mut ctx = pollster::block_on(RenderContext::new(
        window.clone(),
        config.wgpu_present_mode(),
    ));
    let mut ui_ctx = UiContext::new(&window, ctx.device(), ctx.surface_format());
    let mut state = LabState::new(model, cues_path, assets);
    let lights = LightScene::new(DirectionalLight::new(Vec3::NEG_Y, Vec3::ONE, 1.0));
    let started = std::time::Instant::now();
    let mut last = started;

    let result = event_loop.run_closure(move |event, elwt| match event {
        Event::WindowEvent { event, .. } => {
            ui_ctx.handle_window_event(&window, &event);
            match event {
                WindowEvent::CloseRequested => elwt.exit(),
                WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                    ctx.resize(size.width, size.height);
                }
                WindowEvent::RedrawRequested => {
                    let now = std::time::Instant::now();
                    let dt = now.duration_since(last).as_secs_f32().min(0.1);
                    last = now;
                    // The 3D scene is empty; rendering it clears the surface under the UI.
                    let aspect = ctx.config.width as f32 / ctx.config.height.max(1) as f32;
                    let camera = Camera::new(0.9, aspect, 0.1, 100.0);
                    ctx.camera_uniform
                        .update(&camera, Vec3::new(0.0, 0.0, 5.0), Quat::IDENTITY);
                    ctx.update_camera_buffer();
                    let frame = FrameView::new(&camera, &[], &lights);
                    if let Ok(output) = redox_render::acquire_surface_texture(ctx.surface()) {
                        let view = output
                            .texture
                            .create_view(&wgpu::TextureViewDescriptor::default());
                        if let Err(error) = ctx.render_frame_view(&frame, &view) {
                            eprintln!("frame error: {error:?}");
                        }
                        ui_ctx.record_frame(dt);
                        ui_ctx.begin_frame(&window);
                        state.draw(&ui_ctx.egui_ctx, started.elapsed().as_secs_f32());
                        let mut encoder =
                            ctx.device()
                                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                    label: Some("audio_lab_ui"),
                                });
                        ui_ctx.end_frame_and_render(
                            ctx.device(),
                            ctx.queue(),
                            &mut encoder,
                            &window,
                            &view,
                            egui_wgpu::ScreenDescriptor {
                                size_in_pixels: [ctx.config.width, ctx.config.height],
                                pixels_per_point: window.scale_factor() as f32,
                            },
                        );
                        ctx.queue().submit(std::iter::once(encoder.finish()));
                        ctx.queue().present(output);
                    }
                }
                _ => {}
            }
        }
        Event::AboutToWait => window.request_redraw(),
        _ => {}
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("event loop: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Plays the built-in library without an output device and checks the cue rules.
fn selftest() -> Result<(), String> {
    let exists = |_: &str| true;
    let model = LabModel::new(SELFTEST_LIBRARY, &exists);
    if model.has_errors() {
        return Err(format!("test library has errors: {:?}", model.findings()));
    }
    let mut ctx = AudioContext::new();
    let mut selector = CueSelector::new();
    let library = model.library();

    let first = ctx
        .play_cue(library, &mut selector, "step", 0.0)
        .ok_or_else(|| "cue `step` did not play".to_owned())?;
    if !(0.8..=1.0).contains(&first.volume) {
        return Err(format!("volume {} is outside the cue range", first.volume));
    }
    if !(0.95..=1.05).contains(&first.pitch) {
        return Err(format!("pitch {} is outside the cue range", first.pitch));
    }
    if ctx.play_cue(library, &mut selector, "hit", 0.0).is_none() {
        return Err("cue `hit` did not play".to_owned());
    }
    if ctx.play_cue(library, &mut selector, "hit", 0.5).is_some() {
        return Err("cue `hit` played during its cooldown".to_owned());
    }
    if ctx.play_cue(library, &mut selector, "hit", 1.5).is_none() {
        return Err("cue `hit` did not play after its cooldown".to_owned());
    }
    Ok(())
}

/// True when `dir` holds the sound `variant` as written, or with a `.wav` or `.ogg` extension.
/// ASSUMPTION: this file-name convention is a guess, see the module documentation.
fn find_sound(dir: &Path, variant: &str) -> bool {
    dir.join(variant).is_file()
        || dir.join(format!("{variant}.wav")).is_file()
        || dir.join(format!("{variant}.ogg")).is_file()
}

/// True when the sound `name` is available: always without an assets folder, else found in it.
fn sound_exists(assets: &Option<PathBuf>, name: &str) -> bool {
    match assets {
        Some(dir) => find_sound(dir, name),
        None => true,
    }
}

/// Prints the cues and findings of the library file and returns the exit code.
fn check_file(cues_path: &Path, assets: Option<PathBuf>) -> ExitCode {
    let text = match std::fs::read_to_string(cues_path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("cannot read {}: {error}", cues_path.display());
            return ExitCode::FAILURE;
        }
    };
    let exists = |name: &str| sound_exists(&assets, name);
    let model = LabModel::new(&text, &exists);
    for name in model.names() {
        println!("cue {name}");
    }
    for finding in model.findings() {
        let cue = finding.cue.as_deref().unwrap_or("-");
        println!("{:?} [{cue}] {}", finding.severity, finding.message);
    }
    if model.has_errors() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Returns the value that follows `flag` in `args`, if any.
fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let index = args.iter().position(|arg| arg == flag)?;
    args.get(index + 1).map(String::as_str)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--selftest") {
        return match selftest() {
            Ok(()) => {
                println!("audio_lab selftest: ok");
                ExitCode::SUCCESS
            }
            Err(message) => {
                eprintln!("audio_lab selftest: {message}");
                ExitCode::FAILURE
            }
        };
    }
    let assets = flag_value(&args, "--assets").map(PathBuf::from);
    let want_window = args.iter().any(|arg| arg == "--window") || args.is_empty();
    match flag_value(&args, "--cues") {
        Some(path) if !want_window => check_file(Path::new(path), assets),
        Some(path) => {
            let cues_path = PathBuf::from(path);
            match std::fs::read_to_string(&cues_path) {
                Ok(text) => {
                    let exists = |name: &str| sound_exists(&assets, name);
                    let model = LabModel::new(&text, &exists);
                    run_window(model, Some(cues_path), assets)
                }
                Err(error) => {
                    eprintln!("cannot read {}: {error}", cues_path.display());
                    ExitCode::FAILURE
                }
            }
        }
        None if want_window => run_window(
            LabModel::new(SELFTEST_LIBRARY, &|_: &str| true),
            None,
            assets,
        ),
        None => {
            eprintln!(
                "usage: audio_lab [--window] | --selftest | --cues <cues.ron> [--assets <dir>] [--window]"
            );
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OTHER_LIBRARY: &str = r#"[
        (name: "hit", variants: ["h1", "h2"], volume: (1.0, 1.0), pitch: (0.9, 1.1),
         cooldown: 0.0, bus: Sfx, priority: 5),
    ]"#;

    /// Mono sine of `amplitude` and one second at 48 kHz.
    fn sine(amplitude: f32) -> Vec<f32> {
        (0..48_000)
            .map(|i| amplitude * (std::f32::consts::TAU * 997.0 * i as f32 / 48_000.0).sin())
            .collect()
    }

    fn stats(peak_db: f32, lufs: f32, duration: f32) -> AudioStats {
        AudioStats {
            peak_db,
            rms_db: -20.0,
            lufs_integrated: lufs,
            duration,
            clipped_samples: 0,
            dc_offset: 0.0,
            channels_detected: 1,
        }
    }

    #[test]
    fn the_selftest_of_audio_lab_passes_without_an_output_device() {
        assert_eq!(selftest(), Ok(()));
    }

    #[test]
    fn reloading_a_changed_library_updates_the_list() {
        let exists = |_: &str| true;
        let mut model = LabModel::new(SELFTEST_LIBRARY, &exists);
        assert_eq!(model.names(), vec!["step".to_owned(), "hit".to_owned()]);
        assert!(model.reload(OTHER_LIBRARY, &exists));
        assert_eq!(model.names(), vec!["hit".to_owned()]);
        assert!(
            !model.reload(OTHER_LIBRARY, &exists),
            "same text must not reload"
        );
    }

    #[test]
    fn the_listener_distance_matches_the_dragged_position() {
        assert!((listener_distance([0.0, 0.0], [3.0, 4.0]) - 5.0).abs() < 1e-6);
        assert!((listener_distance([1.0, -2.0], [1.0, -2.0])).abs() < 1e-6);
    }

    #[test]
    fn the_bearing_is_zero_ahead_and_positive_to_the_right() {
        assert!(listener_bearing([0.0, 5.0]).abs() < 1e-4);
        assert!((listener_bearing([5.0, 0.0]) - 90.0).abs() < 1e-4);
        assert!((listener_bearing([-5.0, 0.0]) + 90.0).abs() < 1e-4);
    }

    #[test]
    fn a_dragged_point_of_the_plane_maps_back_to_itself_and_stays_in_range() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 300.0));
        let point = [3.0, -7.5];
        let back = plane_to_world(world_to_plane(point, rect), rect);
        assert!((back[0] - point[0]).abs() < 1e-3 && (back[1] - point[1]).abs() < 1e-3);
        let outside = plane_to_world(egui::pos2(900.0, -50.0), rect);
        assert_eq!(outside, [SPATIAL_RANGE, SPATIAL_RANGE]);
    }

    #[test]
    fn the_adaptive_demo_track_fades_its_layers_in_with_the_control() {
        let music = demo_music();
        assert_eq!(
            music.gains(0.0),
            vec![1.0, 0.0, 0.0],
            "only the base layer is on at rest"
        );
        let mid = music.gains(0.45);
        assert!(mid[1] > 0.0 && mid[1] < 1.0 && mid[2] == 0.0);
        assert_eq!(music.gains(1.0)[2], 1.0);
    }

    #[test]
    fn a_full_scale_sine_is_reported_as_clipping() {
        let measured = analyze_samples(&sine(1.0), 1, 48_000);
        let row = analysis_row("tone.wav", Ok((measured, None)), false);
        assert!(row.problems.iter().any(|p| p.starts_with("clipping")));
    }

    #[test]
    fn quiet_music_is_outside_the_loudness_target() {
        let measured = analyze_samples(&sine(0.1), 1, 48_000);
        let problems = problems_for(&measured, None, true);
        assert!(problems.iter().any(|p| p.starts_with("music loudness")));
    }

    #[test]
    fn a_silent_file_is_reported_alone() {
        let measured = analyze_samples(&vec![0.0; 48_000], 1, 48_000);
        assert_eq!(
            problems_for(&measured, None, false),
            vec!["the file is silent".to_owned()]
        );
    }

    #[test]
    fn a_bad_loop_seam_and_a_long_effect_are_problems() {
        let seam = problems_for(&stats(-6.0, -18.0, 2.0), Some(6.0), true);
        assert!(seam.iter().any(|p| p.starts_with("loop seam")));
        let long = problems_for(&stats(-6.0, -18.0, 12.0), None, false);
        assert!(long.iter().any(|p| p.contains("longer than")));
        assert!(problems_for(&stats(-6.0, -18.0, 12.0), None, true).is_empty());
    }

    #[test]
    fn a_file_that_cannot_be_decoded_is_an_error_row_without_problems() {
        let row = analysis_row("bad.wav", Err("cannot decode".to_owned()), false);
        assert!(row.stats.is_none());
        assert_eq!(row.error.as_deref(), Some("cannot decode"));
        assert!(row.problems.is_empty());
    }

    #[test]
    fn a_file_in_a_music_folder_is_music_whatever_the_case() {
        assert!(is_music_path(Path::new("music/theme.ogg")));
        assert!(is_music_path(Path::new("Music/deep/a.wav")));
        assert!(!is_music_path(Path::new("sfx/music_box.wav")));
        assert!(!is_music_path(Path::new("theme.ogg")));
    }

    #[test]
    fn the_model_keeps_the_analysis_rows_until_they_are_replaced() {
        let exists = |_: &str| true;
        let mut model = LabModel::new(SELFTEST_LIBRARY, &exists);
        assert!(model.analysis().is_empty());
        model.set_analysis(vec![analysis_row("e.wav", Err("x".to_owned()), false)]);
        assert_eq!(model.analysis().len(), 1);
        model.set_analysis(Vec::new());
        assert!(model.analysis().is_empty());
    }
}
