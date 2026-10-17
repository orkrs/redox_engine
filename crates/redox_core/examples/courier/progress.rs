//! What the game remembers between runs: the best time and the chosen language.
//!
//! Stored through the engine's `SaveManager` (versioned envelope, backup of the previous file);
//! the payload is a few `key=value` lines that ignore anything they do not understand.

use std::path::PathBuf;

use redox_scene::save::SaveManager;

/// Save slot used for the progress file.
const SLOT: &str = "progress";
/// Version of the payload layout.
const SCHEMA: u32 = 1;
const GAME_VERSION: u32 = 1;

/// Persistent player progress.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    /// Fastest time in which every coin was collected, in seconds.
    pub best_seconds: Option<f32>,
    /// Language code chosen in the menu.
    pub language: Option<String>,
}

impl Progress {
    /// Reads the payload text; unknown keys, bad numbers and negative times are ignored.
    pub fn parse(payload: &str) -> Self {
        let mut progress = Self::default();
        for line in payload.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "best_seconds" => {
                    progress.best_seconds = value
                        .trim()
                        .parse::<f32>()
                        .ok()
                        .filter(|t| t.is_finite() && *t > 0.0);
                }
                "language" => {
                    let code = value.trim();
                    if !code.is_empty() && code.len() <= 8 {
                        progress.language = Some(code.to_string());
                    }
                }
                _ => {}
            }
        }
        progress
    }

    /// The payload text for [`Self::parse`].
    pub fn to_payload(&self) -> String {
        let mut text = String::new();
        if let Some(t) = self.best_seconds {
            text.push_str(&format!("best_seconds={t}\n"));
        }
        if let Some(language) = &self.language {
            text.push_str(&format!("language={language}\n"));
        }
        text
    }

    /// Registers a finished run; returns true when it is a new best time.
    pub fn record_run(&mut self, seconds: f32) -> bool {
        if !seconds.is_finite() || seconds <= 0.0 {
            return false;
        }
        if self.best_seconds.is_none_or(|best| seconds < best) {
            self.best_seconds = Some(seconds);
            return true;
        }
        false
    }

    /// Loads the progress; a missing or damaged file gives the default progress.
    pub fn load(saves: &SaveManager) -> Self {
        match saves.load(SLOT) {
            Ok(loaded) => Self::parse(&loaded.payload),
            Err(e) => {
                log::info!("no usable progress file yet: {e}");
                Self::default()
            }
        }
    }

    /// Saves the progress; failures are logged, the game goes on.
    pub fn save(&self, saves: &SaveManager) {
        if let Err(e) = saves.save(SLOT, "Courier progress", &self.to_payload()) {
            log::warn!("could not save progress: {e}");
        }
    }
}

/// Where the saves live: the user's application data folder, or the temp folder as a fallback.
pub fn save_manager() -> SaveManager {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    SaveManager::new(base.join("RedOxCourier"), SCHEMA, GAME_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_saves(tag: &str) -> SaveManager {
        let dir =
            std::env::temp_dir().join(format!("courier_progress_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        SaveManager::new(dir, SCHEMA, GAME_VERSION)
    }

    #[test]
    fn progress_round_trips_through_the_payload() {
        let progress = Progress {
            best_seconds: Some(41.25),
            language: Some("ru".into()),
        };
        assert_eq!(Progress::parse(&progress.to_payload()), progress);
        assert_eq!(Progress::parse(""), Progress::default());
        assert_eq!(Progress::default().to_payload(), "");
    }

    #[test]
    fn garbage_is_ignored() {
        let text = "nonsense\nbest_seconds=abc\nbest_seconds=-3\nbest_seconds=NaN\nlanguage=\n\
                    future_key=1\nlanguage=toolonglanguagecode\n";
        assert_eq!(Progress::parse(text), Progress::default());
        let mixed = "best_seconds = 12.5 \nlanguage = en\nfuture_key=1";
        assert_eq!(
            Progress::parse(mixed),
            Progress {
                best_seconds: Some(12.5),
                language: Some("en".into())
            }
        );
    }

    #[test]
    fn only_a_faster_finite_run_is_a_record() {
        let mut progress = Progress::default();
        assert!(
            progress.record_run(30.0),
            "the first run is the best so far"
        );
        assert!(!progress.record_run(30.0), "equal is not a record");
        assert!(!progress.record_run(45.0));
        assert!(progress.record_run(29.9));
        assert_eq!(progress.best_seconds, Some(29.9));
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(!progress.record_run(bad));
        }
        assert_eq!(progress.best_seconds, Some(29.9));
    }

    #[test]
    fn progress_survives_a_save_and_load() {
        let saves = temp_saves("roundtrip");
        assert_eq!(Progress::load(&saves), Progress::default(), "no file yet");
        let progress = Progress {
            best_seconds: Some(18.5),
            language: Some("ru".into()),
        };
        progress.save(&saves);
        assert_eq!(Progress::load(&saves), progress);
        let _ = std::fs::remove_dir_all(saves.dir());
    }

    #[test]
    fn a_damaged_file_gives_the_default_progress() {
        let saves = temp_saves("damaged");
        std::fs::create_dir_all(saves.dir()).unwrap();
        std::fs::write(saves.dir().join("progress.save"), "this is not a save").unwrap();
        assert_eq!(Progress::load(&saves), Progress::default());
        let _ = std::fs::remove_dir_all(saves.dir());
    }
}
