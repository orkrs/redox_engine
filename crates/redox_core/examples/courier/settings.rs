//! Player settings: master volume and the four effect switches (F1-F4 in the window title).
//!
//! SPECIFICATION FILE (tests first). The bodies marked `todo!()` are to be implemented so that the
//! tests at the bottom pass; do not change the tests or the signatures. Settings are stored in their
//! own save slot `"settings"` (a few `key=value` lines, unknown keys and bad values ignored, like
//! `progress.rs`) through the same `SaveManager`.
//!
//! Rules:
//! - `master_volume` is clamped to `[0, 1]`; NaN becomes the default 0.8.
//! - `parse` reads keys `volume` (f32), `bloom`, `taa`, `ssao`, `shadows` (`1`/`0`/`true`/`false`,
//!   case-insensitive); anything else keeps the default of that field. Whitespace around keys and
//!   values is ignored.
//! - `to_payload` writes all five keys, volume with two decimals (`volume=0.30`), switches as `1`/`0`,
//!   one `key=value` per line, in the order volume, bloom, taa, ssao, shadows.
//! - `step_volume(delta)` adds `delta` and clamps; the menu uses steps of 0.1. Repeated steps must not
//!   drift: the result is rounded to two decimals.
//! - `toggle(Switch)` flips one switch.
//! - `label_en(Switch)` / `label_ru(Switch)` are the menu captions: "Bloom", "TAA", "SSAO", "Shadows"
//!   and "Блум", "TAA", "SSAO", "Тени".

use redox_scene::save::SaveManager;

const SLOT: &str = "settings";

/// One of the effect switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    Bloom,
    Taa,
    Ssao,
    Shadows,
}

impl Switch {
    /// All switches in menu order.
    #[cfg(test)]
    pub const ALL: [Switch; 4] = [Switch::Bloom, Switch::Taa, Switch::Ssao, Switch::Shadows];
}

/// The settings the player can change.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub master_volume: f32,
    pub bloom: bool,
    pub taa: bool,
    pub ssao: bool,
    pub shadows: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            master_volume: 0.8,
            bloom: true,
            taa: true,
            ssao: true,
            shadows: true,
        }
    }
}

impl Settings {
    /// Reads the payload text (see the module rules).
    pub fn parse(payload: &str) -> Self {
        let mut settings = Self::default();
        for line in payload.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim();
            match key.as_str() {
                "volume" => {
                    if let Some(volume) = value.parse::<f32>().ok().filter(|v| !v.is_nan()) {
                        settings.master_volume = volume.clamp(0.0, 1.0);
                    }
                }
                "bloom" => settings.bloom = parse_flag(value).unwrap_or(settings.bloom),
                "taa" => settings.taa = parse_flag(value).unwrap_or(settings.taa),
                "ssao" => settings.ssao = parse_flag(value).unwrap_or(settings.ssao),
                "shadows" => settings.shadows = parse_flag(value).unwrap_or(settings.shadows),
                _ => {}
            }
        }
        settings
    }

    /// The payload text for [`Self::parse`].
    pub fn to_payload(&self) -> String {
        format!(
            "volume={:.2}\nbloom={}\ntaa={}\nssao={}\nshadows={}\n",
            self.master_volume,
            flag_text(self.bloom),
            flag_text(self.taa),
            flag_text(self.ssao),
            flag_text(self.shadows),
        )
    }

    /// Changes the volume by `delta`, clamped to `[0, 1]` and rounded to two decimals.
    pub fn step_volume(&mut self, delta: f32) {
        let stepped = (self.master_volume + delta).clamp(0.0, 1.0);
        self.master_volume = (stepped * 100.0).round() / 100.0;
    }

    /// Current value of a switch.
    pub fn get(&self, switch: Switch) -> bool {
        match switch {
            Switch::Bloom => self.bloom,
            Switch::Taa => self.taa,
            Switch::Ssao => self.ssao,
            Switch::Shadows => self.shadows,
        }
    }

    /// Flips a switch.
    pub fn toggle(&mut self, switch: Switch) {
        let value = !self.get(switch);
        match switch {
            Switch::Bloom => self.bloom = value,
            Switch::Taa => self.taa = value,
            Switch::Ssao => self.ssao = value,
            Switch::Shadows => self.shadows = value,
        }
    }

    /// Loads the settings; a missing or damaged file gives the defaults.
    pub fn load(saves: &SaveManager) -> Self {
        match saves.load(SLOT) {
            Ok(loaded) => Self::parse(&loaded.payload),
            Err(e) => {
                log::info!("no usable settings file yet: {e}");
                Self::default()
            }
        }
    }

    /// Saves the settings; failures are logged, the game goes on.
    pub fn save(&self, saves: &SaveManager) {
        if let Err(e) = saves.save(SLOT, "Courier settings", &self.to_payload()) {
            log::warn!("could not save settings: {e}");
        }
    }
}

/// Reads `1`/`true` and `0`/`false` (case-insensitive); anything else is `None`.
fn parse_flag(value: &str) -> Option<bool> {
    if value == "1" || value.eq_ignore_ascii_case("true") {
        Some(true)
    } else if value == "0" || value.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

fn flag_text(value: bool) -> &'static str {
    if value { "1" } else { "0" }
}

/// English menu caption of a switch.
pub fn label_en(switch: Switch) -> &'static str {
    match switch {
        Switch::Bloom => "Bloom",
        Switch::Taa => "TAA",
        Switch::Ssao => "SSAO",
        Switch::Shadows => "Shadows",
    }
}

/// Russian menu caption of a switch.
pub fn label_ru(switch: Switch) -> &'static str {
    match switch {
        Switch::Bloom => "Блум",
        Switch::Taa => "TAA",
        Switch::Ssao => "SSAO",
        Switch::Shadows => "Тени",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_saves(tag: &str) -> SaveManager {
        let dir =
            std::env::temp_dir().join(format!("courier_settings_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        SaveManager::new(dir, 1, 1)
    }

    #[test]
    fn defaults_are_loud_enough_and_everything_is_on() {
        let s = Settings::default();
        assert_eq!(s.master_volume, 0.8);
        assert!(Switch::ALL.iter().all(|&sw| s.get(sw)));
    }

    #[test]
    fn the_payload_round_trips() {
        let s = Settings {
            master_volume: 0.3,
            bloom: false,
            taa: true,
            ssao: false,
            shadows: true,
        };
        assert_eq!(Settings::parse(&s.to_payload()), s);
        assert_eq!(Settings::parse(""), Settings::default());
    }

    #[test]
    fn the_payload_has_a_fixed_shape() {
        let s = Settings {
            master_volume: 0.3,
            bloom: false,
            ..Settings::default()
        };
        assert_eq!(
            s.to_payload(),
            "volume=0.30\nbloom=0\ntaa=1\nssao=1\nshadows=1\n"
        );
    }

    #[test]
    fn garbage_and_out_of_range_values_are_handled() {
        let text = "volume=abc\nbloom=maybe\nnonsense\nunknown=1\n";
        assert_eq!(Settings::parse(text), Settings::default());
        assert_eq!(Settings::parse("volume=7").master_volume, 1.0);
        assert_eq!(Settings::parse("volume=-2").master_volume, 0.0);
        assert_eq!(Settings::parse("volume=NaN").master_volume, 0.8);
        let tolerant = Settings::parse(" volume = 0.5 \n BLOOM = FALSE\ntaa=True\nssao=0");
        assert_eq!(tolerant.master_volume, 0.5);
        assert!(!tolerant.bloom && tolerant.taa && !tolerant.ssao && tolerant.shadows);
    }

    #[test]
    fn volume_steps_clamp_and_do_not_drift() {
        let mut s = Settings::default();
        for _ in 0..10 {
            s.step_volume(-0.1);
        }
        assert_eq!(s.master_volume, 0.0);
        s.step_volume(-0.1);
        assert_eq!(s.master_volume, 0.0);
        for _ in 0..10 {
            s.step_volume(0.1);
        }
        assert_eq!(s.master_volume, 1.0);
        s.step_volume(0.1);
        assert_eq!(s.master_volume, 1.0);
        s.master_volume = 0.0;
        for _ in 0..3 {
            s.step_volume(0.1);
        }
        assert_eq!(s.master_volume, 0.3);
    }

    #[test]
    fn toggling_flips_only_that_switch() {
        let mut s = Settings::default();
        s.toggle(Switch::Taa);
        assert!(!s.get(Switch::Taa));
        assert!(s.get(Switch::Bloom) && s.get(Switch::Ssao) && s.get(Switch::Shadows));
        s.toggle(Switch::Taa);
        assert!(s.get(Switch::Taa));
    }

    #[test]
    fn labels_exist_in_both_languages() {
        assert_eq!(label_en(Switch::Bloom), "Bloom");
        assert_eq!(label_ru(Switch::Bloom), "Блум");
        assert_eq!(label_ru(Switch::Shadows), "Тени");
        for sw in Switch::ALL {
            assert!(!label_en(sw).is_empty() && !label_ru(sw).is_empty());
        }
    }

    #[test]
    fn settings_survive_a_save_and_a_load() {
        let saves = temp_saves("roundtrip");
        let s = Settings {
            master_volume: 0.6,
            ssao: false,
            ..Settings::default()
        };
        s.save(&saves);
        assert_eq!(Settings::load(&saves), s);
    }

    #[test]
    fn a_missing_file_gives_the_defaults() {
        assert_eq!(Settings::load(&temp_saves("missing")), Settings::default());
    }
}
