//! Dialogs of the project start: crash recovery and the "open in another session" warning, plus
//! the Preferences window that edits the autosave policy.

use redox_editor_core::autosave::{AutosavePolicy, MAX_INTERVAL_SECS, MIN_INTERVAL_SECS};
use redox_editor_core::recovery::{RecoveryOffer, SessionLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// What the start-up dialog asks about.
#[derive(Clone, Debug, PartialEq)]
pub enum RecoveryPrompt {
    /// A crashed session left an autosave newer than the level.
    Offer(RecoveryOffer),
    /// The project holds a fresh session lock: another editor may have it open.
    Locked(SessionLock),
}

/// The user's answer to a [`RecoveryPrompt`].
#[derive(Clone, Debug, PartialEq)]
pub enum RecoveryAnswer {
    /// Load the autosave into the level.
    Recover(RecoveryOffer),
    /// Move the autosaves into `discarded/`.
    Discard(RecoveryOffer),
    /// Open the project although another session may own it.
    OpenAnyway,
    /// Do not take the recovery; the editor continues without it.
    Cancel,
}

/// Button pressed in the recovery window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecoveryButton {
    Primary,
    Secondary,
    Cancel,
}

/// Modal-style window shown at start-up while a prompt is pending.
#[derive(Debug, Default)]
pub struct RecoveryDialog {
    prompt: Option<RecoveryPrompt>,
}

impl RecoveryDialog {
    /// Creates the dialog with the prompt found at start-up, if any.
    pub fn new(prompt: Option<RecoveryPrompt>) -> Self {
        Self { prompt }
    }

    /// Whether a prompt is waiting for an answer.
    pub fn is_open(&self) -> bool {
        self.prompt.is_some()
    }

    /// Draws the pending prompt and returns the answer once a button is pressed.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<RecoveryAnswer> {
        let prompt = self.prompt.take()?;
        let Some(button) = draw_prompt(ctx, &prompt) else {
            self.prompt = Some(prompt);
            return None;
        };
        Some(match (prompt, button) {
            (RecoveryPrompt::Offer(offer), RecoveryButton::Primary) => {
                RecoveryAnswer::Recover(offer)
            }
            (RecoveryPrompt::Offer(offer), RecoveryButton::Secondary) => {
                RecoveryAnswer::Discard(offer)
            }
            (RecoveryPrompt::Locked(_), RecoveryButton::Primary) => RecoveryAnswer::OpenAnyway,
            (_, RecoveryButton::Cancel) | (RecoveryPrompt::Locked(_), RecoveryButton::Secondary) => {
                RecoveryAnswer::Cancel
            }
        })
    }
}

/// Draws the window for `prompt` and returns the button pressed this frame, if any.
fn draw_prompt(ctx: &egui::Context, prompt: &RecoveryPrompt) -> Option<RecoveryButton> {
    let mut clicked = None;
    egui::Window::new("Recover autosave")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| match prompt {
            RecoveryPrompt::Offer(offer) => {
                ui.label(format!(
                    "The editor closed unexpectedly. An autosave of {} from {} is newer than the level file.",
                    offer.level,
                    format_hhmm(unix_secs(offer.autosave_time))
                ));
                ui.label(match offer.level_time {
                    Some(time) => format!("Level file saved at {}.", format_hhmm(unix_secs(time))),
                    None => "The level file does not exist yet.".to_string(),
                });
                ui.horizontal(|ui| {
                    if ui.button("Recover").clicked() {
                        clicked = Some(RecoveryButton::Primary);
                    }
                    if ui.button("Discard").clicked() {
                        clicked = Some(RecoveryButton::Secondary);
                    }
                    if ui.button("Cancel").clicked() {
                        clicked = Some(RecoveryButton::Cancel);
                    }
                });
            }
            RecoveryPrompt::Locked(lock) => {
                ui.label(format!(
                    "The project is open in another session (pid {}, last heartbeat {}).",
                    lock.pid,
                    format_hhmm(lock.heartbeat)
                ));
                ui.label("Continue only if that session is gone.");
                ui.horizontal(|ui| {
                    if ui.button("Open anyway").clicked() {
                        clicked = Some(RecoveryButton::Primary);
                    }
                    if ui.button("Cancel").clicked() {
                        clicked = Some(RecoveryButton::Cancel);
                    }
                });
            }
        });
    clicked
}

/// Window that edits the autosave policy (Edit > Preferences).
#[derive(Debug, Default)]
pub struct PreferencesWindow {
    open: bool,
    draft: AutosavePolicy,
}

impl PreferencesWindow {
    /// Opens the window with `current` as the starting values.
    pub fn open_with(&mut self, current: AutosavePolicy) {
        self.draft = current;
        self.open = true;
    }

    /// Whether the window is shown.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Draws the window. Returns the new policy when the user presses Save.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<AutosavePolicy> {
        if !self.open {
            return None;
        }
        let mut saved = None;
        let mut cancelled = false;
        let mut open = true;
        egui::Window::new("Preferences")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.heading("Autosave");
                ui.checkbox(&mut self.draft.enabled, "Enabled");
                ui.add(
                    egui::Slider::new(
                        &mut self.draft.interval_secs,
                        MIN_INTERVAL_SECS..=MAX_INTERVAL_SECS,
                    )
                    .text("Interval, s"),
                );
                ui.add(
                    egui::Slider::new(&mut self.draft.every_n_transactions, 1..=1000)
                        .text("Every N transactions"),
                );
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        saved = Some(self.draft.clamped());
                    }
                    if ui.button("Cancel").clicked() {
                        cancelled = true;
                    }
                });
            });
        self.open = open && saved.is_none() && !cancelled;
        saved
    }
}

/// Whole seconds since the Unix epoch of `time`; times before the epoch give 0.
fn unix_secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs())
}

/// `HH:MM` of a Unix time, in UTC (the editor has no time-zone dependency).
pub fn format_hhmm(secs: u64) -> String {
    let minutes_of_day = (secs % 86_400) / 60;
    format!("{:02}:{:02}", minutes_of_day / 60, minutes_of_day % 60)
}
