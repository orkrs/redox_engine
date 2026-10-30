//! Sound lab (AUD-07, step 3).
//!
//! Without arguments (or with `--window`) it opens a window with four tabs: **Cue** (play a cue and
//! see which variant, volume and pitch were chosen, and whether the cooldown or the voice budget
//! stopped it), **Music** (move the control value of an adaptive track and see the gain of each
//! layer), **3D** (move a source around the listener: distance and bearing) and **Check** (the
//! validation findings of the library).
//!
//! `--cues <cues.ron>` loads a cue library (the built-in one when absent), prints its cues and the
//! validation findings, and exits with a failure code when there is an error. `--assets <dir>` also
//! checks that each sound file is in the directory. With `--cues` and without `--window` there is
//! no window.
//!
//! ASSUMPTION: a variant is found as a file named exactly like it, or with a `.wav` or `.ogg`
//! extension. The engine's asset naming was not checked against the loader.
//! `--selftest` runs the built-in check of cue selection without an output device.
//!
//! The window plays no sound files: it shows what the cue selection decides. Registering the sound
//! data and playing it through the output device is left to the game.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use redox_audio::{
    AdaptiveMusic, AudioContext, CueLibrary, CueSelector, Finding, MusicLayer, Severity,
    validate_cue_ron,
};

/// Library used by `--selftest`. Covers a cue with variants and a cue with a cooldown.
const SELFTEST_LIBRARY: &str = r#"[
    (name: "step", variants: ["s1", "s2"], volume: (0.8, 1.0), pitch: (0.95, 1.05),
     cooldown: 0.0, bus: Sfx, priority: 1),
    (name: "hit", variants: ["h1"], volume: (1.0, 1.0), pitch: (0.9, 1.1),
     cooldown: 1.0, bus: Sfx, priority: 5),
]"#;

/// The state of the lab for one cue library: its text, parsed cues and validation findings.
#[derive(Debug, Default)]
pub struct LabModel {
    text: String,
    library: CueLibrary,
    findings: Vec<Finding>,
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

/// The adaptive track shown in the Music tab: a base layer, a mid layer and a high layer.
fn demo_music() -> AdaptiveMusic {
    AdaptiveMusic {
        layers: vec![
            MusicLayer {
                track: "base".to_owned(),
                fade_in_from: 0.0,
                full_from: 0.0,
            },
            MusicLayer {
                track: "mid".to_owned(),
                fade_in_from: 0.3,
                full_from: 0.6,
            },
            MusicLayer {
                track: "high".to_owned(),
                fade_in_from: 0.7,
                full_from: 1.0,
            },
        ],
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Cue,
    Music,
    Spatial,
    Check,
}

/// What the window keeps between frames.
struct LabState {
    model: LabModel,
    audio: AudioContext,
    selector: CueSelector,
    tab: Tab,
    log: Vec<String>,
    music: AdaptiveMusic,
    control: f32,
    source: [f32; 2],
}

impl LabState {
    fn new(model: LabModel) -> Self {
        Self {
            model,
            audio: AudioContext::new(),
            selector: CueSelector::new(),
            tab: Tab::Cue,
            log: Vec::new(),
            music: demo_music(),
            control: 0.0,
            source: [3.0, 4.0],
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
        self.log.push(line);
        if self.log.len() > 200 {
            self.log.remove(0);
        }
    }

    fn draw(&mut self, ctx: &egui::Context, now: f32) {
        egui::Window::new("Audio lab")
            .anchor(egui::Align2::LEFT_TOP, [10.0, 10.0])
            .default_width(640.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.tab, Tab::Cue, "Cue");
                    ui.selectable_value(&mut self.tab, Tab::Music, "Music");
                    ui.selectable_value(&mut self.tab, Tab::Spatial, "3D");
                    ui.selectable_value(&mut self.tab, Tab::Check, "Check");
                });
                ui.separator();
                match self.tab {
                    Tab::Cue => self.cue_tab(ui, now),
                    Tab::Music => self.music_tab(ui),
                    Tab::Spatial => self.spatial_tab(ui),
                    Tab::Check => self.check_tab(ui),
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
        ui.add(egui::Slider::new(&mut self.control, 0.0..=1.0).text("control (tension)"));
        let gains = self.music.gains(self.control);
        for (layer, gain) in self.music.layers.iter().zip(gains) {
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
        ui.label("The listener stands at the origin looking along +Z.");
        ui.add(egui::Slider::new(&mut self.source[0], -20.0..=20.0).text("source x (right)"));
        ui.add(egui::Slider::new(&mut self.source[1], -20.0..=20.0).text("source z (ahead)"));
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

    fn check_tab(&mut self, ui: &mut egui::Ui) {
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

/// Opens the lab window and runs it until it is closed.
fn run_window(model: LabModel) -> ExitCode {
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
    let mut state = LabState::new(model);
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

/// Prints the cues and findings of the library file and returns the exit code.
fn check_file(cues_path: &Path, assets: Option<PathBuf>) -> ExitCode {
    let text = match std::fs::read_to_string(cues_path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("cannot read {}: {error}", cues_path.display());
            return ExitCode::FAILURE;
        }
    };
    let exists = |name: &str| match &assets {
        Some(dir) => find_sound(dir, name),
        None => true,
    };
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
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) => {
                let exists = |name: &str| match &assets {
                    Some(dir) => find_sound(dir, name),
                    None => true,
                };
                run_window(LabModel::new(&text, &exists))
            }
            Err(error) => {
                eprintln!("cannot read {path}: {error}");
                ExitCode::FAILURE
            }
        },
        None if want_window => run_window(LabModel::new(SELFTEST_LIBRARY, &|_: &str| true)),
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
}
