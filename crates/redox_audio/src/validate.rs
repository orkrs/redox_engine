//! Checks a cue library before the game runs (AUD-07, step 1).
//!
//! A typo in a sound file name used to show up only in the log during play. These checks run on
//! the authored library instead. [`Severity::Error`] marks a cue that cannot play as written;
//! [`Severity::Warning`] marks a value that plays but is probably a mistake.
//!
//! Rules that [`CueLibrary::parse`] already enforces (duplicate or empty names, reversed ranges,
//! NaN, negative cooldown) are not repeated here. [`validate_cue_ron`] reports a parse failure as
//! an error finding, so these rules are still checked before the game starts.
//!
//! Rules that cannot occur in the current format are left out: priority is a `u8`, and [`SoundCue`]
//! has no tags and no chains.

use std::collections::HashMap;

use crate::cues::{CueLibrary, SoundCue};

/// Largest linear gain a cue volume may use (about +12 dB).
pub const MAX_LINEAR_GAIN: f32 = 4.0;
/// Largest playback rate a cue pitch may use (two octaves up).
pub const MAX_PLAYBACK_RATE: f32 = 4.0;

/// How serious a [`Finding`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// The cue cannot play correctly as authored.
    Error,
    /// The cue plays, but the value looks like a mistake.
    Warning,
}

/// One problem found in a cue library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// How serious the problem is.
    pub severity: Severity,
    /// The cue the problem belongs to, when it is known.
    pub cue: Option<String>,
    /// Human-readable description of the problem.
    pub message: String,
}

impl Finding {
    /// Creates an error finding for `cue`.
    fn error(cue: &SoundCue, message: String) -> Self {
        Self {
            severity: Severity::Error,
            cue: Some(cue.name.clone()),
            message,
        }
    }

    /// Creates a warning finding for `cue`.
    fn warning(cue: &SoundCue, message: String) -> Self {
        Self {
            severity: Severity::Warning,
            cue: Some(cue.name.clone()),
            message,
        }
    }
}

/// Checks a parsed library against the sound files available through `exists`.
///
/// Returns every finding, not only the first. An empty result means the library is clean.
pub fn validate_cue_library(lib: &CueLibrary, exists: &dyn Fn(&str) -> bool) -> Vec<Finding> {
    let mut findings = Vec::new();
    for cue in lib.cues() {
        check_cue(cue, exists, &mut findings);
    }
    check_duplicate_file_sets(lib, &mut findings);
    findings
}

/// Parses RON text and checks the result. A parse failure becomes one error finding.
///
/// Use this on the authored text, so that duplicate names and other parse-time errors are
/// reported with the rest of the findings.
pub fn validate_cue_ron(text: &str, exists: &dyn Fn(&str) -> bool) -> Vec<Finding> {
    match CueLibrary::parse(text) {
        Ok(lib) => validate_cue_library(&lib, exists),
        Err(error) => vec![Finding {
            severity: Severity::Error,
            cue: None,
            message: error.to_string(),
        }],
    }
}

/// Runs the per-cue checks for one cue.
fn check_cue(cue: &SoundCue, exists: &dyn Fn(&str) -> bool, out: &mut Vec<Finding>) {
    if cue.variants.is_empty() {
        out.push(Finding::error(cue, "cue has no variants".into()));
    }
    for variant in &cue.variants {
        if !exists(variant) {
            out.push(Finding::error(
                cue,
                format!("sound file `{variant}` not found"),
            ));
        }
    }
    if cue.volume.0 < 0.0 || cue.volume.1 > MAX_LINEAR_GAIN {
        out.push(Finding::error(
            cue,
            format!("volume {:?} is outside 0..={MAX_LINEAR_GAIN}", cue.volume),
        ));
    }
    if cue.pitch.0 <= 0.0 || cue.pitch.1 > MAX_PLAYBACK_RATE {
        out.push(Finding::error(
            cue,
            format!("pitch {:?} is outside (0..={MAX_PLAYBACK_RATE}]", cue.pitch),
        ));
    }
    if cue.variants.len() == 1 && cue.pitch.0 == cue.pitch.1 {
        out.push(Finding::warning(
            cue,
            "one variant without a pitch range repeats identically".into(),
        ));
    }
}

/// Warns about cues that use exactly the same set of sound files.
///
/// The second cue of each such pair is reported and names the first one.
fn check_duplicate_file_sets(lib: &CueLibrary, out: &mut Vec<Finding>) {
    let mut seen: HashMap<Vec<&str>, &SoundCue> = HashMap::new();
    for cue in lib.cues() {
        if cue.variants.is_empty() {
            continue;
        }
        let mut key: Vec<&str> = cue.variants.iter().map(String::as_str).collect();
        key.sort_unstable();
        key.dedup();
        match seen.get(&key) {
            Some(first) => out.push(Finding::warning(
                cue,
                format!("uses the same sound files as `{}`", first.name),
            )),
            None => {
                seen.insert(key, cue);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_exist(_: &str) -> bool {
        true
    }

    /// RON text for one cue with cooldown 0 and the default bus.
    fn cue_text(name: &str, variants: &[&str], volume: (f32, f32), pitch: (f32, f32)) -> String {
        let list = variants
            .iter()
            .map(|v| format!("\"{v}\""))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "(name: \"{name}\", variants: [{list}], volume: {volume:?}, pitch: {pitch:?}, \
             cooldown: 0.0, bus: Sfx, priority: 1)"
        )
    }

    /// Joins cue texts into one RON list.
    fn list(cues: &[String]) -> String {
        format!("[{}]", cues.join(",\n"))
    }

    fn library(text: &str) -> CueLibrary {
        CueLibrary::parse(text).expect("test library parses")
    }

    fn errors(findings: &[Finding]) -> Vec<&Finding> {
        findings
            .iter()
            .filter(|f| f.severity == Severity::Error)
            .collect()
    }

    fn warnings(findings: &[Finding]) -> Vec<&Finding> {
        findings
            .iter()
            .filter(|f| f.severity == Severity::Warning)
            .collect()
    }

    #[test]
    fn a_missing_file_is_an_error() {
        let lib = library(&list(&[cue_text(
            "boom",
            &["b1", "missing"],
            (1.0, 1.0),
            (1.0, 1.0),
        )]));
        let findings = validate_cue_library(&lib, &|name| name != "missing");
        let errs = errors(&findings);
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].cue.as_deref(), Some("boom"));
        assert!(errs[0].message.contains("missing"));
    }

    #[test]
    fn a_duplicate_cue_name_is_an_error() {
        let text = list(&[
            cue_text("step", &["s1"], (1.0, 1.0), (1.0, 1.0)),
            cue_text("step", &["s2"], (1.0, 1.0), (1.0, 1.0)),
        ]);
        let findings = validate_cue_ron(&text, &all_exist);
        let errs = errors(&findings);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("duplicate"));
    }

    #[test]
    fn a_negative_cooldown_is_an_error() {
        let text = "[(name: \"step\", variants: [\"s1\"], volume: (1.0, 1.0), pitch: (1.0, 1.0), \
                    cooldown: -1.0, bus: Sfx, priority: 1)]";
        let findings = validate_cue_ron(text, &all_exist);
        assert_eq!(errors(&findings).len(), 1);
    }

    #[test]
    fn an_empty_variant_list_is_an_error() {
        let lib = library(&list(&[cue_text("silent", &[], (1.0, 1.0), (1.0, 1.0))]));
        let findings = validate_cue_library(&lib, &all_exist);
        let errs = errors(&findings);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("no variants"));
    }

    #[test]
    fn a_volume_outside_the_range_is_an_error() {
        let too_loud = library(&list(&[cue_text(
            "a",
            &["s1", "s2"],
            (0.5, 5.0),
            (1.0, 1.0),
        )]));
        let negative = library(&list(&[cue_text(
            "b",
            &["s1", "s2"],
            (-0.1, 1.0),
            (1.0, 1.0),
        )]));
        assert_eq!(
            errors(&validate_cue_library(&too_loud, &all_exist)).len(),
            1
        );
        assert_eq!(
            errors(&validate_cue_library(&negative, &all_exist)).len(),
            1
        );
    }

    #[test]
    fn a_pitch_at_or_below_zero_is_an_error() {
        let lib = library(&list(&[cue_text(
            "a",
            &["s1", "s2"],
            (1.0, 1.0),
            (0.0, 1.0),
        )]));
        assert_eq!(errors(&validate_cue_library(&lib, &all_exist)).len(), 1);
    }

    #[test]
    fn a_single_variant_without_pitch_range_is_a_warning() {
        let lib = library(&list(&[cue_text("tick", &["t1"], (1.0, 1.0), (1.0, 1.0))]));
        let findings = validate_cue_library(&lib, &all_exist);
        assert!(errors(&findings).is_empty());
        assert_eq!(warnings(&findings).len(), 1);
    }

    #[test]
    fn two_cues_with_the_same_files_warn() {
        let lib = library(&list(&[
            cue_text("first", &["a", "b"], (1.0, 1.0), (0.9, 1.1)),
            cue_text("second", &["b", "a"], (1.0, 1.0), (0.9, 1.1)),
        ]));
        let findings = validate_cue_library(&lib, &all_exist);
        let warns = warnings(&findings);
        assert_eq!(warns.len(), 1);
        assert_eq!(warns[0].cue.as_deref(), Some("second"));
        assert!(warns[0].message.contains("first"));
    }

    #[test]
    fn a_valid_library_has_no_findings() {
        let lib = library(&list(&[
            cue_text("step", &["s1", "s2", "s3"], (0.8, 1.0), (0.95, 1.05)),
            cue_text("hit", &["h1", "h2"], (1.0, 1.0), (0.9, 1.1)),
        ]));
        assert!(validate_cue_library(&lib, &all_exist).is_empty());
    }

    #[test]
    fn all_findings_are_reported_not_only_the_first() {
        let lib = library(&list(&[
            cue_text("a", &["x1", "x2"], (1.0, 1.0), (1.0, 1.0)),
            cue_text("b", &[], (1.0, 1.0), (1.0, 1.0)),
        ]));
        let findings = validate_cue_library(&lib, &|_| false);
        // Two missing files, plus one empty cue.
        assert!(findings.len() >= 3, "got {findings:?}");
    }

    #[test]
    fn a_parse_failure_is_one_error_finding() {
        let findings = validate_cue_ron("[(name: \"oops\"", &all_exist);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Error);
        assert_eq!(findings[0].cue, None);
    }
}
