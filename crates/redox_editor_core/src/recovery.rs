//! Session lock and crash recovery.
//!
//! `<project>/.redox/session.lock` records who edits a project and when its heartbeat last moved.
//! When a project opens and the lock is stale while a newer autosave exists, the autosave is
//! offered for recovery. A fresh lock means another session may be running; the editor then asks
//! for an explicit confirmation. Liveness of the pid is not checked (no `unsafe`, no new crates).

use crate::autosave::{autosave_dir, file_stem_for, list_autosaves};
use crate::log::EditorLog;
use redox_scene::ofpa::write_atomic;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// File name of the session lock inside `<project>/.redox/`.
pub const LOCK_FILE: &str = "session.lock";
/// Folder inside `<project>/.redox/autosave/` that receives discarded or damaged autosaves.
pub const DISCARDED_DIR: &str = "discarded";
/// A lock is never considered stale before this many seconds have passed.
pub const MIN_STALE_SECS: u64 = 30;

/// Contents of `session.lock`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionLock {
    /// Process id of the editor that holds the lock.
    pub pid: u32,
    /// Unix time when the session started.
    pub started: u64,
    /// Project root the session works on.
    pub project: String,
    /// Name of the level being edited.
    pub level: String,
    /// Unix time of the last heartbeat.
    pub heartbeat: u64,
}

/// An autosave that is newer than its level and should be offered to the user.
#[derive(Clone, Debug, PartialEq)]
pub struct RecoveryOffer {
    /// Name of the level.
    pub level: String,
    /// File of the level, or `None` for a level never saved. Saving writes here.
    pub level_path: Option<PathBuf>,
    /// The autosave file to load from.
    pub autosave_path: PathBuf,
    /// Modification time of the autosave.
    pub autosave_time: SystemTime,
    /// Modification time of the level file, if it exists.
    pub level_time: Option<SystemTime>,
}

/// Outcome of [`check_recovery`].
#[derive(Clone, Debug, PartialEq)]
pub enum RecoveryCheck {
    /// Nothing to recover.
    Clean,
    /// A crashed session left a newer autosave.
    Offer(RecoveryOffer),
    /// The lock is fresh: another session may own the project. Continue only after confirmation.
    LockedByOtherSession(SessionLock),
}

/// The user's answer to a [`RecoveryOffer`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryChoice {
    /// Load the autosave into the level.
    Recover,
    /// Move the autosaves of the level into `discarded/`.
    Discard,
}

/// Inputs of [`check_recovery`].
#[derive(Clone, Copy, Debug)]
pub struct RecoveryQuery<'a> {
    /// Project root that holds `.redox/`.
    pub project_root: &'a Path,
    /// Name of the level that is about to open.
    pub level: &'a str,
    /// File of the level, if it has one.
    pub level_path: Option<&'a Path>,
    /// Current Unix time.
    pub now: u64,
    /// Autosave interval of the current policy.
    pub interval_secs: u32,
}

/// Path of `session.lock` for a project root.
pub fn lock_path(project_root: &Path) -> PathBuf {
    project_root.join(".redox").join(LOCK_FILE)
}

/// Reads the lock; `Ok(None)` when there is no lock file.
pub fn read_lock(project_root: &Path) -> io::Result<Option<SessionLock>> {
    let text = match fs::read_to_string(lock_path(project_root)) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    ron::from_str::<SessionLock>(&text)
        .map(Some)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))
}

/// Writes the lock atomically.
pub fn write_lock(project_root: &Path, lock: &SessionLock) -> io::Result<()> {
    let text = ron::ser::to_string_pretty(lock, ron::ser::PrettyConfig::default())
        .map_err(|err| io::Error::other(err.to_string()))?;
    write_atomic(lock_path(project_root), text.as_bytes())
}

/// Removes the lock; a missing lock is not an error.
pub fn remove_lock(project_root: &Path) -> io::Result<()> {
    match fs::remove_file(lock_path(project_root)) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Seconds without a heartbeat after which a lock is stale: twice the autosave interval, but
/// never less than [`MIN_STALE_SECS`].
pub fn stale_threshold_secs(interval_secs: u32) -> u64 {
    (u64::from(interval_secs) * 2).max(MIN_STALE_SECS)
}

/// Moves one file into `<dir>/discarded/<now>/`, keeping its name.
fn move_to_discarded(dir: &Path, file: &Path, now: u64) -> io::Result<()> {
    let Some(name) = file.file_name() else {
        return Err(io::Error::other("autosave has no file name"));
    };
    let target_dir = dir.join(DISCARDED_DIR).join(now.to_string());
    fs::create_dir_all(&target_dir)?;
    fs::rename(file, target_dir.join(name))
}

/// Decides whether a crashed session left something to recover for `query.level`.
///
/// `is_valid` tells whether the bytes of an autosave parse as a level. A damaged newest autosave
/// is moved to `discarded/`, logged, and not offered.
pub fn check_recovery(
    query: &RecoveryQuery<'_>,
    is_valid: &dyn Fn(&[u8]) -> bool,
    log: &EditorLog,
) -> RecoveryCheck {
    let lock = match read_lock(query.project_root) {
        Ok(Some(lock)) => lock,
        Ok(None) => return RecoveryCheck::Clean,
        Err(err) => {
            log.warn(format!("session.lock is unreadable and is ignored: {err}"));
            return RecoveryCheck::Clean;
        }
    };
    let age = query.now.saturating_sub(lock.heartbeat);
    if age < stale_threshold_secs(query.interval_secs) {
        return RecoveryCheck::LockedByOtherSession(lock);
    }
    let dir = autosave_dir(query.project_root);
    let mut autosaves = list_autosaves(&dir, &file_stem_for(query.level));
    let Some((_, newest)) = autosaves.pop() else {
        return RecoveryCheck::Clean;
    };
    let Ok(autosave_time) = fs::metadata(&newest).and_then(|meta| meta.modified()) else {
        log.warn(format!("cannot read the time of {}", newest.display()));
        return RecoveryCheck::Clean;
    };
    let level_time = query
        .level_path
        .and_then(|path| fs::metadata(path).and_then(|meta| meta.modified()).ok());
    if level_time.is_some_and(|level| autosave_time <= level) {
        return RecoveryCheck::Clean;
    }
    let bytes = match fs::read(&newest) {
        Ok(bytes) => bytes,
        Err(err) => {
            log.warn(format!("cannot read autosave {}: {err}", newest.display()));
            return RecoveryCheck::Clean;
        }
    };
    if !is_valid(&bytes) {
        log.warn(format!(
            "autosave {} is damaged; it is moved to {DISCARDED_DIR} and not offered",
            newest.display()
        ));
        if let Err(err) = move_to_discarded(&dir, &newest, query.now) {
            log.error(format!("cannot move the damaged autosave: {err}"));
        }
        return RecoveryCheck::Clean;
    }
    RecoveryCheck::Offer(RecoveryOffer {
        level: query.level.to_string(),
        level_path: query.level_path.map(Path::to_path_buf),
        autosave_path: newest,
        autosave_time,
        level_time,
    })
}

/// Carries out the user's choice.
///
/// With [`RecoveryChoice::Recover`] the autosave bytes are returned, and the caller loads them into
/// the level (it stays bound to its original path). With [`RecoveryChoice::Discard`] every
/// autosave of the level moves into `discarded/<now>/` and `None` is returned. Both remove the
/// stale lock; the new session writes its own.
pub fn apply_recovery(
    project_root: &Path,
    offer: &RecoveryOffer,
    choice: RecoveryChoice,
    now: u64,
) -> io::Result<Option<Vec<u8>>> {
    match choice {
        RecoveryChoice::Recover => {
            let bytes = fs::read(&offer.autosave_path)?;
            remove_lock(project_root)?;
            Ok(Some(bytes))
        }
        RecoveryChoice::Discard => {
            let dir = autosave_dir(project_root);
            for (_, path) in list_autosaves(&dir, &file_stem_for(&offer.level)) {
                move_to_discarded(&dir, &path, now)?;
            }
            remove_lock(project_root)?;
            Ok(None)
        }
    }
}
