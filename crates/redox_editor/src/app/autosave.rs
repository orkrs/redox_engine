//! Autosave and crash recovery of the editor application: the level source, the start-up
//! question, the answers of the recovery dialog and the close-time session end.

use super::EditorApp;
use redox_editor_core::EditorContext;
use redox_editor_core::EditorLog;
use redox_editor_core::autosave::{
    AutosavePolicy, AutosaveService, AutosaveSource, Clock, SystemClock, WriteMode,
};
use redox_editor_core::recovery::{
    RecoveryCheck, RecoveryChoice, RecoveryOffer, RecoveryQuery, apply_recovery, check_recovery,
};
use redox_editor_core::settings::{EditorSettings, SETTINGS_FILE};
use redox_editor_ui::{RecoveryAnswer, RecoveryPrompt};
use std::path::{Path, PathBuf};

/// Autosave source of the level open in the editor.
///
/// OFPA worlds are not autosaved in EDT-A4 (📌 in the report): `scene_path` is `None`, so the
/// level never counts as dirty for autosave and nothing is written. A scene level from EDT-A2
/// sets `scene_path` and supplies its own bytes.
pub(crate) struct OpenLevel<'a> {
    core: &'a EditorContext,
    name: &'a str,
    scene_path: Option<PathBuf>,
}

impl AutosaveSource for OpenLevel<'_> {
    fn is_dirty(&self) -> bool {
        self.scene_path.is_some() && self.core.undo.lock().is_dirty()
    }

    fn level_name(&self) -> String {
        self.name.to_string()
    }

    fn level_path(&self) -> Option<PathBuf> {
        self.scene_path.clone()
    }

    fn snapshot_bytes(&self) -> Result<Vec<u8>, String> {
        let world = self.core.world.lock();
        let registry = self.core.registry.lock();
        registry
            .to_ron(&world)
            .map(String::into_bytes)
            .map_err(|err| err.to_string())
    }
}

/// Level name of an opened world folder (`worlds/arena` gives `arena`).
pub(crate) fn level_name_of(world_root: &str) -> String {
    Path::new(world_root)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("world")
        .to_string()
}

/// Creates the autosave service for an existing world folder. Without such a folder (the demo
/// scene) there is nowhere to keep `.redox/`, so no service is created.
pub(crate) fn create_service(
    world_root: &str,
    settings: &EditorSettings,
    log: &EditorLog,
) -> Option<AutosaveService> {
    let root = PathBuf::from(world_root);
    if !root.is_dir() {
        return None;
    }
    Some(AutosaveService::new(
        root,
        settings.autosave,
        Box::new(SystemClock),
        log.clone(),
        WriteMode::Background,
    ))
}

/// The question for the start of this session, if a crashed or concurrent session left one.
pub(crate) fn startup_prompt(
    service: Option<&AutosaveService>,
    level: &str,
    log: &EditorLog,
) -> Option<RecoveryPrompt> {
    let service = service?;
    let query = RecoveryQuery {
        project_root: service.project_root(),
        level,
        level_path: None,
        now: SystemClock.now_secs(),
        interval_secs: service.policy().interval_secs,
    };
    // Until scene files exist, an autosave counts as valid when it looks like a RON tuple.
    let is_valid = |bytes: &[u8]| bytes.starts_with(b"(");
    match check_recovery(&query, &is_valid, log) {
        RecoveryCheck::Clean => None,
        RecoveryCheck::Offer(offer) => Some(RecoveryPrompt::Offer(offer)),
        RecoveryCheck::LockedByOtherSession(lock) => Some(RecoveryPrompt::Locked(lock)),
    }
}

/// Takes the session lock for `level`. Failures are logged; the editor keeps running.
pub(crate) fn begin_session(
    autosave: &mut Option<AutosaveService>,
    core: &EditorContext,
    level: &str,
) {
    let Some(service) = autosave.as_mut() else {
        return;
    };
    let source = OpenLevel {
        core,
        name: level,
        scene_path: None,
    };
    if let Err(err) = service.begin_session(&source) {
        core.log.error(format!("session lock could not be written: {err}"));
    }
}

impl EditorApp {
    /// Runs one autosave cycle from the main loop, once the session lock is held.
    pub(super) fn tick_autosave(&mut self) {
        let Some(service) = self.autosave.as_mut() else {
            return;
        };
        if !service.is_session_active() {
            return;
        }
        let source = OpenLevel {
            core: &self.core,
            name: &self.level_name,
            scene_path: None,
        };
        let transactions = self.core.undo.lock().len() as u64;
        service.tick(&source, transactions);
    }

    /// Ends the session at a normal close: the lock and autosaves go when the level is clean.
    pub(super) fn finish_autosave(&mut self) {
        let Some(service) = self.autosave.as_mut() else {
            return;
        };
        let source = OpenLevel {
            core: &self.core,
            name: &self.level_name,
            scene_path: None,
        };
        let dirty = source.is_dirty();
        if let Err(err) = service.finish_session(dirty) {
            self.core
                .log
                .warn(format!("session lock could not be removed: {err}"));
        }
    }

    /// Carries out the answer of the recovery dialog.
    ///
    /// Recover: the autosave is read, and loading it into the level waits for the scene format
    /// (the limit of EDT-A4 for OFPA worlds). Discard: the autosaves move to `discarded/`.
    /// Cancel: the editor runs without autosave for this start, so no existing autosave or lock
    /// is touched.
    pub(super) fn apply_recovery_answer(&mut self, answer: RecoveryAnswer) {
        match answer {
            RecoveryAnswer::Recover(offer) => self.resolve_offer(&offer, RecoveryChoice::Recover),
            RecoveryAnswer::Discard(offer) => self.resolve_offer(&offer, RecoveryChoice::Discard),
            RecoveryAnswer::OpenAnyway => {
                self.core
                    .log
                    .warn("project opened although another session may own it");
                begin_session(&mut self.autosave, &self.core, &self.level_name);
            }
            RecoveryAnswer::Cancel => {
                self.core
                    .log
                    .info("recovery declined: autosave is off for this session");
                self.autosave = None;
            }
        }
    }

    /// Applies a recover or discard choice to an offer, then takes the session lock.
    fn resolve_offer(&mut self, offer: &RecoveryOffer, choice: RecoveryChoice) {
        let Some(service) = self.autosave.as_ref() else {
            return;
        };
        let result = apply_recovery(
            service.project_root(),
            offer,
            choice,
            SystemClock.now_secs(),
        );
        match result {
            Ok(Some(bytes)) => self.core.log.warn(format!(
                "autosave of {} read ({} bytes); loading it waits for the scene format (EDT-A4 📌)",
                offer.level,
                bytes.len()
            )),
            Ok(None) => self.core.log.info(format!(
                "autosaves of {} moved to discarded/",
                offer.level
            )),
            Err(err) => self
                .core
                .log
                .error(format!("recovery failed: {err}")),
        }
        begin_session(&mut self.autosave, &self.core, &self.level_name);
    }

    /// Stores new autosave preferences, writes them to `editor_settings.ron` and applies them.
    pub(super) fn apply_preferences(&mut self, policy: AutosavePolicy) {
        self.settings.autosave = policy;
        if let Err(err) = self.settings.save(Path::new(SETTINGS_FILE)) {
            self.core
                .log
                .error(format!("settings were not saved: {err}"));
        }
        if let Some(service) = self.autosave.as_mut() {
            service.set_policy(policy);
        }
    }
}
