//! Sound lab (AUD-07, step 3), without a window.
//!
//! `--cues <cues.ron>` loads a cue library, prints its cues and the validation findings, and exits
//! with a failure code when there is an error. `--assets <dir>` also checks that each sound file is
//! in the directory.
//!
//! ASSUMPTION: a variant is found as a file named exactly like it, or with a `.wav` or `.ogg`
//! extension. The engine's asset naming was not checked against the loader.
//! `--selftest` runs the built-in check of cue selection without an output device.
//!
//! The egui window with the Cue, Music, 3D and Analysis tabs is not built yet. Its logic
//! (`LabModel`, listener distance) is here so that the window can be added on top.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use redox_audio::{AudioContext, CueLibrary, CueSelector, Finding, Severity, validate_cue_ron};

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
/// Used by the 3D tab to show the distance of the dragged source. Not called yet.
#[allow(dead_code)]
fn listener_distance(listener: [f32; 2], source: [f32; 2]) -> f32 {
    (source[0] - listener[0]).hypot(source[1] - listener[1])
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
    match flag_value(&args, "--cues") {
        Some(path) => {
            let assets = flag_value(&args, "--assets").map(PathBuf::from);
            check_file(Path::new(path), assets)
        }
        None => {
            eprintln!("usage: audio_lab --selftest | --cues <cues.ron> [--assets <dir>]");
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
        assert!(!model.reload(OTHER_LIBRARY, &exists), "same text must not reload");
    }

    #[test]
    fn the_listener_distance_matches_the_dragged_position() {
        assert!((listener_distance([0.0, 0.0], [3.0, 4.0]) - 5.0).abs() < 1e-6);
        assert!((listener_distance([1.0, -2.0], [1.0, -2.0])).abs() < 1e-6);
    }
}
