//! Navigation model of the Courier settings screen (rows, selection, changing values, captions).
//!
//! SPECIFICATION FILE (tests first). Bodies marked `todo!()` are to be implemented so that the tests
//! at the bottom pass; do not change the tests or the signatures. The screen itself (`ui.rs`) and its
//! wiring into `main.rs` are built on top of this model (see the task brief).
//!
//! Rules:
//! - Rows in order: Volume, Bloom, TAA, SSAO, Shadows, Back (`ROWS`). The selection starts on the
//!   first row and wraps around in both directions.
//! - `adjust(settings, direction)` (left = -1, right = +1, anything else = 0 and does nothing):
//!   on Volume it steps the volume by 0.1 per call; on a switch row any non-zero direction flips the
//!   switch; on Back nothing happens. Returns true when `settings` changed (a volume step against
//!   the 0 or 1 end changes nothing and returns false).
//! - `activate(settings)` (Enter / A button): on a switch row flips it and returns `Changed`; on
//!   Volume does nothing (`None`); on Back returns `Back`.
//! - `caption(row, settings, russian)` is the text of a row:
//!     Volume  -> "Volume: 80%"        / "Громкость: 80%"   (percent rounded to an integer)
//!     switch  -> "Bloom: on"          / "Блум: вкл"   ("off" / "выкл" when off; names as `label_en` /
//!                `label_ru` in `settings.rs`)
//!     Back    -> "Back"               / "Назад"

#![allow(dead_code)]

use crate::settings::{Settings, Switch};

/// One row of the settings screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Volume,
    Switch(Switch),
    Back,
}

/// What activating a row did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    /// Nothing happened.
    None,
    /// A setting changed and should be applied and saved.
    Changed,
    /// The player left the screen.
    Back,
}

/// Rows of the screen in order.
pub const ROWS: [Row; 6] = [
    Row::Volume,
    Row::Switch(Switch::Bloom),
    Row::Switch(Switch::Taa),
    Row::Switch(Switch::Ssao),
    Row::Switch(Switch::Shadows),
    Row::Back,
];

/// Selection state of the settings screen.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsMenu {
    selected: usize,
}

impl SettingsMenu {
    /// A menu with the first row selected.
    pub fn new() -> Self {
        Self::default()
    }

    /// Index of the selected row in [`ROWS`].
    pub fn selected(&self) -> usize {
        todo!()
    }

    /// The selected row.
    pub fn selected_row(&self) -> Row {
        todo!()
    }

    /// Moves the selection by `delta` rows with wrap-around (`delta` may be any integer).
    pub fn move_selection(&mut self, delta: i32) {
        let _ = delta;
        todo!()
    }

    /// Left / right on the selected row (see the module rules). True when `settings` changed.
    pub fn adjust(&self, settings: &mut Settings, direction: i32) -> bool {
        let _ = (settings, direction);
        todo!()
    }

    /// Enter on the selected row (see the module rules).
    pub fn activate(&self, settings: &mut Settings) -> MenuAction {
        let _ = settings;
        todo!()
    }
}

/// The caption of `row` for the current settings.
pub fn caption(row: Row, settings: &Settings, russian: bool) -> String {
    let _ = (row, settings, russian);
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_selection_starts_on_volume_and_wraps_both_ways() {
        let mut menu = SettingsMenu::new();
        assert_eq!(menu.selected_row(), Row::Volume);
        menu.move_selection(-1);
        assert_eq!(menu.selected_row(), Row::Back, "wraps to the last row");
        menu.move_selection(1);
        assert_eq!(menu.selected_row(), Row::Volume);
        menu.move_selection(7);
        assert_eq!(menu.selected(), 1, "7 rows forward from 0 on 6 rows");
        menu.move_selection(-13);
        assert_eq!(menu.selected(), 0, "13 rows back from 1");
    }

    #[test]
    fn left_and_right_step_the_volume() {
        let mut s = Settings::default();
        let menu = SettingsMenu::new();
        assert!(menu.adjust(&mut s, -1));
        assert_eq!(s.master_volume, 0.7);
        assert!(menu.adjust(&mut s, 1));
        assert!(menu.adjust(&mut s, 1));
        assert_eq!(s.master_volume, 0.9);
        assert!(!menu.adjust(&mut s, 0), "no direction, no change");
        assert_eq!(s.master_volume, 0.9);
    }

    #[test]
    fn the_volume_stops_at_both_ends_without_reporting_a_change() {
        let mut s = Settings::default();
        let menu = SettingsMenu::new();
        for _ in 0..3 {
            menu.adjust(&mut s, 1);
        }
        assert_eq!(s.master_volume, 1.0);
        assert!(!menu.adjust(&mut s, 1));
        for _ in 0..10 {
            menu.adjust(&mut s, -1);
        }
        assert_eq!(s.master_volume, 0.0);
        assert!(!menu.adjust(&mut s, -1));
    }

    #[test]
    fn switches_flip_with_either_direction_and_with_enter() {
        let mut s = Settings::default();
        let mut menu = SettingsMenu::new();
        menu.move_selection(1);
        assert_eq!(menu.selected_row(), Row::Switch(Switch::Bloom));
        assert!(menu.adjust(&mut s, 1));
        assert!(!s.bloom);
        assert!(menu.adjust(&mut s, -1));
        assert!(s.bloom);
        assert_eq!(menu.activate(&mut s), MenuAction::Changed);
        assert!(!s.bloom);
        assert!(s.taa && s.ssao && s.shadows, "other switches untouched");
    }

    #[test]
    fn enter_on_volume_does_nothing_and_on_back_leaves() {
        let mut s = Settings::default();
        let mut menu = SettingsMenu::new();
        assert_eq!(menu.activate(&mut s), MenuAction::None);
        assert_eq!(s, Settings::default());
        menu.move_selection(-1);
        assert_eq!(menu.activate(&mut s), MenuAction::Back);
        assert!(!menu.adjust(&mut s, 1), "left/right on Back does nothing");
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn captions_in_both_languages() {
        let mut s = Settings::default();
        s.master_volume = 0.8;
        s.bloom = false;
        assert_eq!(caption(Row::Volume, &s, false), "Volume: 80%");
        assert_eq!(caption(Row::Volume, &s, true), "Громкость: 80%");
        assert_eq!(caption(Row::Switch(Switch::Bloom), &s, false), "Bloom: off");
        assert_eq!(caption(Row::Switch(Switch::Bloom), &s, true), "Блум: выкл");
        assert_eq!(caption(Row::Switch(Switch::Taa), &s, false), "TAA: on");
        assert_eq!(caption(Row::Switch(Switch::Shadows), &s, true), "Тени: вкл");
        assert_eq!(caption(Row::Back, &s, false), "Back");
        assert_eq!(caption(Row::Back, &s, true), "Назад");
        s.master_volume = 0.35;
        assert_eq!(caption(Row::Volume, &s, false), "Volume: 35%");
    }

    #[test]
    fn every_row_has_a_caption() {
        let s = Settings::default();
        for row in ROWS {
            assert!(!caption(row, &s, false).is_empty());
            assert!(!caption(row, &s, true).is_empty());
        }
    }
}
