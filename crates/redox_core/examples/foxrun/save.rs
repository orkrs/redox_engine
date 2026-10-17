//! The best score and the language, saved between runs through the engine's `SaveManager`.

use std::path::PathBuf;

use redox_scene::save::SaveManager;

const SLOT: &str = "foxfire";
const SCHEMA: u32 = 1;
const GAME_VERSION: u32 = 1;

/// What is remembered.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    pub best_score: u32,
    pub language: Option<String>,
}

impl Progress {
    /// Reads the payload text; unknown keys and bad numbers are ignored.
    pub fn parse(payload: &str) -> Self {
        let mut progress = Self::default();
        for line in payload.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "best_score" => progress.best_score = value.trim().parse().unwrap_or(0),
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

    pub fn to_payload(&self) -> String {
        let mut text = format!("best_score={}\n", self.best_score);
        if let Some(language) = &self.language {
            text.push_str(&format!("language={language}\n"));
        }
        text
    }

    /// Registers a finished run; returns true for a new best score.
    pub fn record(&mut self, score: u32) -> bool {
        if score > self.best_score {
            self.best_score = score;
            return true;
        }
        false
    }

    pub fn load(saves: &SaveManager) -> Self {
        saves
            .load(SLOT)
            .map(|loaded| Self::parse(&loaded.payload))
            .unwrap_or_default()
    }

    pub fn save(&self, saves: &SaveManager) {
        if let Err(e) = saves.save(SLOT, "Foxfire progress", &self.to_payload()) {
            log::warn!("could not save progress: {e}");
        }
    }
}

pub fn save_manager() -> SaveManager {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    SaveManager::new(base.join("RedOxFoxfire"), SCHEMA, GAME_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_round_trips_and_ignores_garbage() {
        let progress = Progress {
            best_score: 4200,
            language: Some("ru".into()),
        };
        assert_eq!(Progress::parse(&progress.to_payload()), progress);
        assert_eq!(
            Progress::parse("best_score=abc\nfoo\nlanguage=\nx=1"),
            Progress::default()
        );
    }

    #[test]
    fn only_a_higher_score_is_a_record() {
        let mut progress = Progress::default();
        assert!(progress.record(100));
        assert!(!progress.record(100));
        assert!(!progress.record(50));
        assert!(progress.record(101));
        assert_eq!(progress.best_score, 101);
    }

    #[test]
    fn progress_survives_a_save_and_a_missing_file_is_empty() {
        let dir = std::env::temp_dir().join(format!("foxfire_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let saves = SaveManager::new(&dir, SCHEMA, GAME_VERSION);
        assert_eq!(Progress::load(&saves), Progress::default());
        let progress = Progress {
            best_score: 777,
            language: Some("en".into()),
        };
        progress.save(&saves);
        assert_eq!(Progress::load(&saves), progress);
        let _ = std::fs::remove_dir_all(dir);
    }
}
