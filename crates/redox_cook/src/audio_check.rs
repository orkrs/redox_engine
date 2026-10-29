//! `redox_cook audio-check` (AUD-07, step 2): checks sound files and an optional cue library.
//!
//! Every `.wav` and `.ogg` file under the folder is decoded and measured. A file counts as music
//! when one of its folders is named `music` (any case, any depth). Music gets the music loudness
//! target and the loop seam check. Everything else is a sound effect.
//!
//! ASSUMPTION: the folder name `music` is the only marker of music, because the asset format has
//! no loop flag yet. ASSUMPTION: a seam window of 50 ms at each end of a clip.
//!
//! Problems are warnings (clipping, loudness outside the target, a seam over 3 dB, an effect longer
//! than 10 s, silence) or errors (a file that cannot be decoded, a cue library error). The command
//! fails only on errors. `--json` prints the report as JSON, with `null` for values that are not
//! measured.

use std::path::{Path, PathBuf};

use redox_audio::{
    AudioStats, Finding, Severity, analyze_samples, decode_bytes, loop_seam_error,
    validate_cue_ron,
};

/// Default loudness target for music in LUFS.
const DEFAULT_TARGET_LUFS_MUSIC: f32 = -16.0;
/// Default loudness target for sound effects in LUFS.
const DEFAULT_TARGET_LUFS_SFX: f32 = -18.0;
/// Default allowed distance from a loudness target, in LU.
const DEFAULT_TOLERANCE_LU: f32 = 3.0;
/// A peak above this (in dBFS) is reported as clipping.
const CLIPPING_PEAK_DB: f32 = -0.1;
/// A loop seam that differs by more than this (in dB) is reported.
const SEAM_LIMIT_DB: f32 = 3.0;
/// Length of the window at each end of a clip that the seam check compares. ASSUMPTION: 50 ms.
const SEAM_WINDOW_SECONDS: f32 = 0.05;
/// Sound effects longer than this (in seconds) are reported.
const SFX_MAX_SECONDS: f32 = 10.0;

/// A parsed `audio-check` command line.
#[derive(Debug, PartialEq)]
pub struct AudioCheckJob {
    /// Folder with the sound files.
    pub dir: PathBuf,
    /// Optional cue library (RON) checked against the folder.
    pub cues: Option<PathBuf>,
    /// Loudness target for music, in LUFS.
    pub target_lufs_music: f32,
    /// Loudness target for sound effects, in LUFS.
    pub target_lufs_sfx: f32,
    /// Allowed distance from a loudness target, in LU.
    pub tolerance: f32,
    /// Print the report as JSON instead of text.
    pub json: bool,
}

/// How serious one problem with a file is.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    /// Error or warning.
    pub severity: Severity,
    /// Description for the author.
    pub message: String,
}

/// The result for one sound file.
#[derive(Clone, Debug, PartialEq)]
pub struct FileEntry {
    /// Path relative to the checked folder, with `/` separators.
    pub path: String,
    /// True for a file in a `music` folder.
    pub music: bool,
    /// Measurements, or `None` when the file could not be read or decoded.
    pub stats: Option<AudioStats>,
    /// Loop seam difference in dB. Only for music.
    pub seam_db: Option<f32>,
    /// Everything found wrong with the file.
    pub problems: Vec<Problem>,
}

/// The whole report of one `audio-check` run.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    /// One entry per sound file, sorted by path.
    pub files: Vec<FileEntry>,
    /// Findings of the cue library, when one was given.
    pub cue_findings: Vec<Finding>,
}

impl Report {
    /// Number of errors in files and in the cue library.
    pub fn error_count(&self) -> usize {
        self.count(Severity::Error)
    }

    /// Number of warnings in files and in the cue library.
    pub fn warning_count(&self) -> usize {
        self.count(Severity::Warning)
    }

    /// Counts problems and cue findings with the given severity.
    fn count(&self, severity: Severity) -> usize {
        let files = self
            .files
            .iter()
            .flat_map(|entry| &entry.problems)
            .filter(|problem| problem.severity == severity)
            .count();
        let cues = self
            .cue_findings
            .iter()
            .filter(|finding| finding.severity == severity)
            .count();
        files + cues
    }
}

/// Parses the arguments after `audio-check`.
///
/// # Errors
/// Returns a message for a missing folder, an unknown option, a missing value or a value that is
/// not a number.
pub fn parse_audio_check(args: &[&str]) -> Result<AudioCheckJob, String> {
    let [dir, flags @ ..] = args else {
        return Err("audio-check needs <assets-dir>".into());
    };
    let mut job = AudioCheckJob {
        dir: dir.into(),
        cues: None,
        target_lufs_music: DEFAULT_TARGET_LUFS_MUSIC,
        target_lufs_sfx: DEFAULT_TARGET_LUFS_SFX,
        tolerance: DEFAULT_TOLERANCE_LU,
        json: false,
    };
    let mut rest = flags.iter().copied();
    while let Some(flag) = rest.next() {
        match flag {
            "--json" => job.json = true,
            "--cues" => job.cues = Some(value(&mut rest, flag)?.into()),
            "--target-lufs" => job.target_lufs_music = number(value(&mut rest, flag)?, flag)?,
            "--target-lufs-sfx" => job.target_lufs_sfx = number(value(&mut rest, flag)?, flag)?,
            "--tolerance" => job.tolerance = number(value(&mut rest, flag)?, flag)?,
            other => return Err(format!("unknown option: {other}")),
        }
    }
    if job.tolerance < 0.0 {
        return Err("--tolerance must not be negative".into());
    }
    Ok(job)
}

/// Takes the value that follows `flag`.
fn value<'a>(rest: &mut impl Iterator<Item = &'a str>, flag: &str) -> Result<&'a str, String> {
    rest.next()
        .ok_or_else(|| format!("{flag} needs a value"))
}

/// Parses a finite number for `flag`.
fn number(text: &str, flag: &str) -> Result<f32, String> {
    text.parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("{flag}: `{text}` is not a number"))
}

/// Checks the folder and the optional cue library, and returns the report.
///
/// # Errors
/// Returns a message when the folder or the cue file cannot be read.
pub fn check(job: &AudioCheckJob) -> Result<Report, String> {
    let paths = collect_sound_files(&job.dir)?;
    let files = paths
        .iter()
        .map(|path| measure_file(&job.dir, path, job))
        .collect();
    let cue_findings = match &job.cues {
        Some(path) => {
            let text =
                std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
            validate_cue_ron(&text, &|name: &str| sound_exists(&job.dir, name))
        }
        None => Vec::new(),
    };
    Ok(Report { files, cue_findings })
}

/// Runs the check, prints the report and returns an error when the report has errors.
///
/// # Errors
/// Returns a message when the check cannot run or when the report has at least one error.
pub fn run(job: &AudioCheckJob) -> Result<(), String> {
    let report = check(job)?;
    if job.json {
        println!("{}", to_json(&report));
    } else {
        print_text(&report);
    }
    match report.error_count() {
        0 => Ok(()),
        errors => Err(format!("{errors} error(s) found by audio-check")),
    }
}

/// Lists every `.wav` and `.ogg` file below `dir`, sorted.
fn collect_sound_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    walk(dir, &mut found)?;
    found.sort();
    Ok(found)
}

/// Adds the sound files below `dir` to `found`.
fn walk(dir: &Path, found: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            walk(&path, found)?;
        } else if is_sound_file(&path) {
            found.push(path);
        }
    }
    Ok(())
}

/// True for a file with a `.wav` or `.ogg` extension (any case).
fn is_sound_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("wav") || ext.eq_ignore_ascii_case("ogg"))
}

/// True when `dir` holds the sound `variant` as written, or with a `.wav` or `.ogg` extension.
/// ASSUMPTION: this file-name convention is a guess, see the module documentation.
fn sound_exists(dir: &Path, variant: &str) -> bool {
    dir.join(variant).is_file()
        || dir.join(format!("{variant}.wav")).is_file()
        || dir.join(format!("{variant}.ogg")).is_file()
}

/// True when a folder between `dir` and the file is named `music`.
fn is_music(path: &Path, dir: &Path) -> bool {
    let relative = path.strip_prefix(dir).unwrap_or(path);
    relative.parent().is_some_and(|parent| {
        parent
            .components()
            .any(|part| part.as_os_str().to_string_lossy().eq_ignore_ascii_case("music"))
    })
}

/// Decodes and measures one file. A file that cannot be read or decoded gets an error.
fn measure_file(dir: &Path, path: &Path, job: &AudioCheckJob) -> FileEntry {
    let relative = path
        .strip_prefix(dir)
        .unwrap_or(path)
        .display()
        .to_string()
        .replace('\\', "/");
    let music = is_music(path, dir);
    let decoded = std::fs::read(path)
        .map_err(|e| format!("cannot read: {e}"))
        .and_then(|bytes| decode_bytes(&bytes).map_err(|e| e.to_string()));
    let audio = match decoded {
        Ok(audio) => audio,
        Err(message) => {
            return FileEntry {
                path: relative,
                music,
                stats: None,
                seam_db: None,
                problems: vec![Problem {
                    severity: Severity::Error,
                    message,
                }],
            };
        }
    };
    let stats = analyze_samples(&audio.samples, audio.channels, audio.sample_rate);
    let window = (audio.sample_rate as f32 * SEAM_WINDOW_SECONDS) as usize;
    let seam_db = music.then(|| loop_seam_error(&audio.samples, audio.channels, window));
    let problems = problems_for(&stats, seam_db, music, job);
    FileEntry {
        path: relative,
        music,
        stats: Some(stats),
        seam_db,
        problems,
    }
}

/// The warnings for one measured file.
fn problems_for(
    stats: &AudioStats,
    seam_db: Option<f32>,
    music: bool,
    job: &AudioCheckJob,
) -> Vec<Problem> {
    let mut problems = Vec::new();
    let mut warn = |message: String| {
        problems.push(Problem {
            severity: Severity::Warning,
            message,
        });
    };
    if stats.is_silent() {
        warn("the file is silent".into());
    } else {
        let (kind, target) = if music {
            ("music", job.target_lufs_music)
        } else {
            ("sound", job.target_lufs_sfx)
        };
        if (stats.lufs_integrated - target).abs() > job.tolerance {
            warn(format!(
                "{kind} loudness {:.2} LUFS is outside {target} +/- {} LU",
                stats.lufs_integrated, job.tolerance
            ));
        }
    }
    if stats.peak_db > CLIPPING_PEAK_DB {
        warn(format!("clipping: peak {:.2} dBFS", stats.peak_db));
    }
    if let Some(seam) = seam_db
        && seam > SEAM_LIMIT_DB
    {
        warn(format!("loop seam differs by {seam:.2} dB"));
    }
    if !music && stats.duration > SFX_MAX_SECONDS {
        warn(format!("{:.2} s is longer than {SFX_MAX_SECONDS} s", stats.duration));
    }
    problems
}

/// Name of a severity as written in the text and JSON output.
fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    }
}

/// Prints the report as text.
fn print_text(report: &Report) {
    for entry in &report.files {
        match &entry.stats {
            Some(stats) => {
                let kind = if entry.music { "music" } else { "sound" };
                let seam = entry
                    .seam_db
                    .map_or_else(String::new, |db| format!(", seam {db:.2} dB"));
                println!(
                    "{}: {kind}, peak {:.2} dBFS, RMS {:.2} dBFS, {:.2} LUFS, {:.2} s{seam}",
                    entry.path, stats.peak_db, stats.rms_db, stats.lufs_integrated, stats.duration
                );
            }
            None => println!("{}: not measured", entry.path),
        }
        for problem in &entry.problems {
            println!("  {}: {}", severity_name(problem.severity), problem.message);
        }
    }
    for finding in &report.cue_findings {
        let cue = finding.cue.as_deref().unwrap_or("-");
        println!(
            "cues: {}: [{cue}] {}",
            severity_name(finding.severity),
            finding.message
        );
    }
    println!(
        "{} file(s), {} error(s), {} warning(s)",
        report.files.len(),
        report.error_count(),
        report.warning_count()
    );
}

/// Quotes and escapes a string for JSON.
fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A number as JSON. A value that is not finite is written as `null`.
fn json_number(value: f32) -> String {
    if value.is_finite() {
        format!("{value}")
    } else {
        "null".into()
    }
}

/// A JSON object for one file.
fn file_json(entry: &FileEntry) -> String {
    let stats = match &entry.stats {
        Some(s) => format!(
            "\"peak_db\":{},\"rms_db\":{},\"lufs\":{},\"duration\":{},\"clipped_samples\":{},\"dc_offset\":{},\"channels\":{}",
            json_number(s.peak_db),
            json_number(s.rms_db),
            json_number(s.lufs_integrated),
            json_number(s.duration),
            s.clipped_samples,
            json_number(s.dc_offset),
            s.channels_detected
        ),
        None => "\"peak_db\":null,\"rms_db\":null,\"lufs\":null,\"duration\":null,\"clipped_samples\":null,\"dc_offset\":null,\"channels\":null".into(),
    };
    let seam = entry
        .seam_db
        .map_or_else(|| "null".to_owned(), json_number);
    let problems: Vec<String> = entry
        .problems
        .iter()
        .map(|p| {
            format!(
                "{{\"severity\":{},\"message\":{}}}",
                json_string(severity_name(p.severity)),
                json_string(&p.message)
            )
        })
        .collect();
    format!(
        "{{\"path\":{},\"music\":{},{stats},\"seam_db\":{seam},\"problems\":[{}]}}",
        json_string(&entry.path),
        entry.music,
        problems.join(",")
    )
}

/// A JSON object for one cue finding.
fn finding_json(finding: &Finding) -> String {
    let cue = finding
        .cue
        .as_deref()
        .map_or_else(|| "null".to_owned(), json_string);
    format!(
        "{{\"severity\":{},\"cue\":{cue},\"message\":{}}}",
        json_string(severity_name(finding.severity)),
        json_string(&finding.message)
    )
}

/// The whole report as one JSON object.
pub fn to_json(report: &Report) -> String {
    let files: Vec<String> = report.files.iter().map(file_json).collect();
    let cues: Vec<String> = report.cue_findings.iter().map(finding_json).collect();
    format!(
        "{{\"files\":[{}],\"cues\":[{}],\"errors\":{},\"warnings\":{}}}",
        files.join(","),
        cues.join(","),
        report.error_count(),
        report.warning_count()
    )
}

/// Writes a 16-bit mono PCM WAV file.
#[cfg(test)]
fn wav_bytes(samples: &[i16], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_audio::analyze::SILENCE_DB;

    fn job() -> AudioCheckJob {
        parse_audio_check(&["assets"]).expect("defaults parse")
    }

    fn stats(peak_db: f32, lufs: f32, rms_db: f32, duration: f32) -> AudioStats {
        AudioStats {
            peak_db,
            rms_db,
            lufs_integrated: lufs,
            duration,
            clipped_samples: 0,
            dc_offset: 0.0,
            channels_detected: 1,
        }
    }

    #[test]
    fn parses_the_defaults_and_every_option() {
        let plain = job();
        assert_eq!(plain.dir, PathBuf::from("assets"));
        assert_eq!(plain.target_lufs_music, -16.0);
        assert_eq!(plain.target_lufs_sfx, -18.0);
        assert_eq!(plain.tolerance, 3.0);
        assert!(!plain.json && plain.cues.is_none());
        let full = parse_audio_check(&[
            "assets",
            "--cues",
            "c.ron",
            "--target-lufs",
            "-14",
            "--target-lufs-sfx",
            "-20",
            "--tolerance",
            "1.5",
            "--json",
        ])
        .expect("full line parses");
        assert_eq!(full.cues, Some(PathBuf::from("c.ron")));
        assert_eq!(full.target_lufs_music, -14.0);
        assert_eq!(full.target_lufs_sfx, -20.0);
        assert_eq!(full.tolerance, 1.5);
        assert!(full.json);
    }

    #[test]
    fn rejects_bad_command_lines() {
        assert!(parse_audio_check(&[]).is_err());
        assert!(parse_audio_check(&["assets", "--bogus"]).is_err());
        assert!(parse_audio_check(&["assets", "--cues"]).is_err());
        assert!(parse_audio_check(&["assets", "--tolerance", "x"]).is_err());
        assert!(parse_audio_check(&["assets", "--tolerance", "-1"]).is_err());
    }

    #[test]
    fn a_clipped_file_is_a_warning() {
        let problems = problems_for(&stats(-0.05, -18.0, -22.0, 1.0), None, false, &job());
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].severity, Severity::Warning);
        assert!(problems[0].message.contains("clipping"));
    }

    #[test]
    fn music_outside_its_loudness_target_is_a_warning() {
        let problems = problems_for(&stats(-3.0, -10.0, -13.0, 30.0), Some(0.5), true, &job());
        assert_eq!(problems.len(), 1);
        assert!(problems[0].message.contains("music loudness"));
    }

    #[test]
    fn a_sound_within_its_target_has_no_problem() {
        let problems = problems_for(&stats(-3.0, -18.0, -21.0, 1.0), None, false, &job());
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn a_long_effect_and_a_bad_seam_are_warnings() {
        let long = problems_for(&stats(-3.0, -18.0, -21.0, 12.0), None, false, &job());
        assert!(long.iter().any(|p| p.message.contains("longer")));
        let seam = problems_for(&stats(-3.0, -16.0, -19.0, 30.0), Some(5.0), true, &job());
        assert!(seam.iter().any(|p| p.message.contains("seam")));
        let fine = problems_for(&stats(-3.0, -16.0, -19.0, 30.0), Some(1.0), true, &job());
        assert!(fine.is_empty(), "{fine:?}");
    }

    #[test]
    fn a_silent_file_is_a_warning_and_skips_the_loudness_check() {
        let silent = stats(SILENCE_DB, SILENCE_DB, SILENCE_DB, 1.0);
        let problems = problems_for(&silent, None, false, &job());
        assert_eq!(problems.len(), 1);
        assert!(problems[0].message.contains("silent"));
    }

    #[test]
    fn music_is_a_file_in_a_music_folder() {
        let dir = Path::new("assets");
        assert!(is_music(Path::new("assets/music/town.ogg"), dir));
        assert!(is_music(Path::new("assets/Music/deep/a.wav"), dir));
        assert!(!is_music(Path::new("assets/sfx/music.wav"), dir));
        assert!(!is_music(Path::new("assets/step.wav"), dir));
    }

    #[test]
    fn a_directory_check_reports_a_bad_file_as_an_error() {
        let dir = std::env::temp_dir().join(format!(
            "redox_cook_audio_check_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(dir.join("music")).expect("temp dir");
        std::fs::write(dir.join("bad.wav"), b"not a sound file").expect("write bad");
        let tone: Vec<i16> = (0..48_000)
            .map(|n| {
                let t = n as f32 / 48_000.0;
                (0.5 * (2.0 * std::f32::consts::PI * 997.0 * t).sin() * 32_767.0) as i16
            })
            .collect();
        std::fs::write(dir.join("music").join("tone.wav"), wav_bytes(&tone, 48_000))
            .expect("write tone");
        let job = AudioCheckJob {
            dir: dir.clone(),
            ..job()
        };
        let report = check(&job);
        let _ = std::fs::remove_dir_all(&dir);
        let report = report.expect("check runs");
        assert_eq!(report.files.len(), 2);
        assert_eq!(report.error_count(), 1);
        let tone_entry = report
            .files
            .iter()
            .find(|entry| entry.path == "music/tone.wav")
            .expect("tone measured");
        assert!(tone_entry.music);
        assert!(tone_entry.stats.is_some() && tone_entry.seam_db.is_some());
        let bad = report
            .files
            .iter()
            .find(|entry| entry.path == "bad.wav")
            .expect("bad file listed");
        assert!(bad.stats.is_none());
    }

    #[test]
    fn json_strings_are_escaped() {
        assert_eq!(json_string("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
        assert_eq!(json_string("\u{1}"), "\"\\u0001\"");
    }

    /// A JSON value, read back by the test to check the report.
    #[derive(Debug, PartialEq)]
    enum Json {
        Null,
        Bool(bool),
        Num(f64),
        Str(String),
        Arr(Vec<Json>),
        Obj(Vec<(String, Json)>),
    }

    /// Minimal JSON reader for the report format.
    struct Reader<'a> {
        bytes: &'a [u8],
        pos: usize,
    }

    impl Reader<'_> {
        fn skip_ws(&mut self) {
            while self.bytes.get(self.pos).is_some_and(u8::is_ascii_whitespace) {
                self.pos += 1;
            }
        }

        fn eat(&mut self, byte: u8) -> Result<(), String> {
            self.skip_ws();
            if self.bytes.get(self.pos) == Some(&byte) {
                self.pos += 1;
                Ok(())
            } else {
                Err(format!("expected `{}` at {}", byte as char, self.pos))
            }
        }

        fn literal(&mut self, word: &str, value: Json) -> Result<Json, String> {
            let matches = self
                .bytes
                .get(self.pos..)
                .is_some_and(|rest| rest.starts_with(word.as_bytes()));
            if matches {
                self.pos += word.len();
                Ok(value)
            } else {
                Err(format!("bad literal at {}", self.pos))
            }
        }

        fn value(&mut self) -> Result<Json, String> {
            self.skip_ws();
            match self.bytes.get(self.pos).copied() {
                Some(b'{') => self.object(),
                Some(b'[') => self.array(),
                Some(b'"') => self.string().map(Json::Str),
                Some(b'n') => self.literal("null", Json::Null),
                Some(b't') => self.literal("true", Json::Bool(true)),
                Some(b'f') => self.literal("false", Json::Bool(false)),
                Some(_) => self.number(),
                None => Err("unexpected end".into()),
            }
        }

        fn number(&mut self) -> Result<Json, String> {
            let start = self.pos;
            while self
                .bytes
                .get(self.pos)
                .is_some_and(|b| b"+-.0123456789eE".contains(b))
            {
                self.pos += 1;
            }
            let text = std::str::from_utf8(&self.bytes[start..self.pos]).map_err(|e| e.to_string())?;
            text.parse::<f64>()
                .map(Json::Num)
                .map_err(|_| format!("bad number `{text}`"))
        }

        fn string(&mut self) -> Result<String, String> {
            self.eat(b'"')?;
            let mut out: Vec<u8> = Vec::new();
            loop {
                let byte = *self.bytes.get(self.pos).ok_or("unterminated string")?;
                self.pos += 1;
                match byte {
                    b'"' => break,
                    b'\\' => {
                        let escape = *self.bytes.get(self.pos).ok_or("bad escape")?;
                        self.pos += 1;
                        match escape {
                            b'"' | b'\\' | b'/' => out.push(escape),
                            b'n' => out.push(b'\n'),
                            b'r' => out.push(b'\r'),
                            b't' => out.push(b'\t'),
                            b'u' => {
                                let hex = self.bytes.get(self.pos..self.pos + 4).ok_or("bad \\u")?;
                                let text = std::str::from_utf8(hex).map_err(|e| e.to_string())?;
                                let code = u32::from_str_radix(text, 16).map_err(|e| e.to_string())?;
                                self.pos += 4;
                                let c = char::from_u32(code).ok_or("bad code point")?;
                                let mut buf = [0u8; 4];
                                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                            }
                            other => return Err(format!("unknown escape {}", other as char)),
                        }
                    }
                    other => out.push(other),
                }
            }
            String::from_utf8(out).map_err(|e| e.to_string())
        }

        fn object(&mut self) -> Result<Json, String> {
            self.eat(b'{')?;
            let mut members = Vec::new();
            self.skip_ws();
            if self.bytes.get(self.pos) == Some(&b'}') {
                self.pos += 1;
                return Ok(Json::Obj(members));
            }
            loop {
                let key = self.string()?;
                self.eat(b':')?;
                members.push((key, self.value()?));
                self.skip_ws();
                match self.bytes.get(self.pos).copied() {
                    Some(b',') => self.pos += 1,
                    Some(b'}') => {
                        self.pos += 1;
                        return Ok(Json::Obj(members));
                    }
                    _ => return Err(format!("expected `,` or `}}` at {}", self.pos)),
                }
            }
        }

        fn array(&mut self) -> Result<Json, String> {
            self.eat(b'[')?;
            let mut items = Vec::new();
            self.skip_ws();
            if self.bytes.get(self.pos) == Some(&b']') {
                self.pos += 1;
                return Ok(Json::Arr(items));
            }
            loop {
                items.push(self.value()?);
                self.skip_ws();
                match self.bytes.get(self.pos).copied() {
                    Some(b',') => self.pos += 1,
                    Some(b']') => {
                        self.pos += 1;
                        return Ok(Json::Arr(items));
                    }
                    _ => return Err(format!("expected `,` or `]` at {}", self.pos)),
                }
            }
        }
    }

    /// Parses a whole JSON document.
    fn parse_json(text: &str) -> Result<Json, String> {
        let mut reader = Reader {
            bytes: text.as_bytes(),
            pos: 0,
        };
        let value = reader.value()?;
        reader.skip_ws();
        if reader.pos == reader.bytes.len() {
            Ok(value)
        } else {
            Err(format!("trailing text at {}", reader.pos))
        }
    }

    /// The member `key` of a JSON object. Panics in a test when it is missing.
    fn field<'a>(value: &'a Json, key: &str) -> &'a Json {
        match value {
            Json::Obj(members) => members
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, v)| v)
                .unwrap_or_else(|| panic!("missing field {key}")),
            other => panic!("not an object: {other:?}"),
        }
    }

    /// The item at `index` of a JSON array.
    fn item(value: &Json, index: usize) -> &Json {
        match value {
            Json::Arr(items) => &items[index],
            other => panic!("not an array: {other:?}"),
        }
    }

    fn sample_report() -> Report {
        Report {
            files: vec![
                FileEntry {
                    path: "music/a \"q\".ogg".into(),
                    music: true,
                    stats: Some(AudioStats {
                        peak_db: -0.5,
                        rms_db: -20.25,
                        lufs_integrated: -16.5,
                        duration: 2.5,
                        clipped_samples: 3,
                        dc_offset: 0.125,
                        channels_detected: 2,
                    }),
                    seam_db: Some(1.5),
                    problems: vec![Problem {
                        severity: Severity::Warning,
                        message: "clipping: peak -0.50 dBFS".into(),
                    }],
                },
                FileEntry {
                    path: "bad.wav".into(),
                    music: false,
                    stats: None,
                    seam_db: None,
                    problems: vec![Problem {
                        severity: Severity::Error,
                        message: "cannot decode".into(),
                    }],
                },
            ],
            cue_findings: vec![Finding {
                severity: Severity::Error,
                cue: Some("step".into()),
                message: "sound file `x` not found".into(),
            }],
        }
    }

    #[test]
    fn the_json_report_roundtrips() {
        let json = to_json(&sample_report());
        let root = parse_json(&json).expect("the report is valid JSON");
        assert_eq!(field(&root, "errors"), &Json::Num(2.0));
        assert_eq!(field(&root, "warnings"), &Json::Num(1.0));
        let files = field(&root, "files");
        let first = item(files, 0);
        assert_eq!(field(first, "path"), &Json::Str("music/a \"q\".ogg".into()));
        assert_eq!(field(first, "music"), &Json::Bool(true));
        assert_eq!(field(first, "peak_db"), &Json::Num(-0.5));
        assert_eq!(field(first, "lufs"), &Json::Num(-16.5));
        assert_eq!(field(first, "channels"), &Json::Num(2.0));
        assert_eq!(field(first, "seam_db"), &Json::Num(1.5));
        let bad = item(files, 1);
        assert_eq!(field(bad, "peak_db"), &Json::Null);
        assert_eq!(field(bad, "seam_db"), &Json::Null);
        let cue = item(field(&root, "cues"), 0);
        assert_eq!(field(cue, "cue"), &Json::Str("step".into()));
        assert_eq!(field(cue, "severity"), &Json::Str("error".into()));
    }
}
