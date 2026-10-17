//! Save slots: versioned, crash-safe save files.
//!
//! A slot is one RON file `<dir>/<slot>.save` holding an envelope (schema version, game
//! version, timestamp, label) and an opaque text payload, usually
//! [`redox_reflect::TypeRegistry::to_ron`] output. Writes go through a temporary file and a
//! rename, and the previous file is kept as `<slot>.bak`, so a crash or a full disk never
//! destroys the last good save.
//!
//! Older files are upgraded on load by chained migrations registered with
//! [`SaveManager::with_migration`]; each one turns the payload of version `n` into
//! version `n + 1`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::{SceneError, SceneResult};

/// Longest accepted slot name.
pub const MAX_SLOT_NAME: usize = 64;

/// Upgrades a payload from schema version `n` to `n + 1`.
pub type Migration = fn(&str) -> SceneResult<String>;

#[derive(Serialize, Deserialize)]
struct Envelope {
    schema_version: u32,
    game_version: u32,
    saved_at_unix: u64,
    label: String,
    payload: String,
}

/// What a slot contains, without its payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotInfo {
    /// Slot name (file stem).
    pub slot: String,
    /// Player-facing label chosen at save time.
    pub label: String,
    /// Seconds since the Unix epoch when the slot was written.
    pub saved_at_unix: u64,
    /// Schema version the file was written with.
    pub schema_version: u32,
    /// Game version recorded at save time.
    pub game_version: u32,
}

/// A loaded save, already migrated to the current schema version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedSave {
    /// Metadata of the slot.
    pub info: SlotInfo,
    /// The (migrated) payload.
    pub payload: String,
}

/// Reads and writes save slots in one directory.
pub struct SaveManager {
    dir: PathBuf,
    game_version: u32,
    current_schema: u32,
    migrations: Vec<(u32, Migration)>,
}

impl SaveManager {
    /// Manages slots in `dir`. `current_schema` is the schema version new saves are written
    /// with; `game_version` is recorded for display only.
    pub fn new(dir: impl Into<PathBuf>, current_schema: u32, game_version: u32) -> Self {
        Self {
            dir: dir.into(),
            game_version,
            current_schema,
            migrations: Vec::new(),
        }
    }

    /// Registers the upgrade from schema `from_version` to `from_version + 1`.
    pub fn with_migration(mut self, from_version: u32, migration: Migration) -> Self {
        self.migrations.push((from_version, migration));
        self
    }

    /// Directory holding the slots.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Slot names are file names, so they are restricted to `[A-Za-z0-9_-]` (no path
    /// separators or dots) and at most [`MAX_SLOT_NAME`] characters.
    pub fn is_valid_slot_name(slot: &str) -> bool {
        !slot.is_empty()
            && slot.len() <= MAX_SLOT_NAME
            && slot
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    }

    fn slot_path(&self, slot: &str) -> SceneResult<PathBuf> {
        if !Self::is_valid_slot_name(slot) {
            return Err(SceneError::InvalidCell(format!(
                "invalid save slot name: {slot:?}"
            )));
        }
        Ok(self.dir.join(format!("{slot}.save")))
    }

    /// Writes `payload` to `slot`, keeping the previous file as a backup.
    pub fn save(&self, slot: &str, label: &str, payload: &str) -> SceneResult<SlotInfo> {
        let path = self.slot_path(slot)?;
        let saved_at_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let envelope = Envelope {
            schema_version: self.current_schema,
            game_version: self.game_version,
            saved_at_unix,
            label: label.to_owned(),
            payload: payload.to_owned(),
        };
        let text = ron::ser::to_string_pretty(&envelope, ron::ser::PrettyConfig::new())?;
        write_replacing(&path, text.as_bytes())?;
        Ok(info_of(slot, &envelope))
    }

    fn read_envelope(path: &Path) -> SceneResult<Envelope> {
        let text = fs::read_to_string(path)?;
        Ok(ron::from_str(&text)?)
    }

    /// Loads a slot and migrates its payload to the current schema.
    ///
    /// Fails for unknown slots, damaged files, files written by a newer schema than this
    /// build understands, and gaps in the migration chain.
    pub fn load(&self, slot: &str) -> SceneResult<LoadedSave> {
        let path = self.slot_path(slot)?;
        let mut envelope = Self::read_envelope(&path)?;
        if envelope.schema_version > self.current_schema {
            return Err(SceneError::InvalidCell(format!(
                "save {slot:?} uses schema {} but this build only knows up to {}",
                envelope.schema_version, self.current_schema
            )));
        }
        let mut payload = envelope.payload.clone();
        while envelope.schema_version < self.current_schema {
            let from = envelope.schema_version;
            let Some((_, migrate)) = self.migrations.iter().find(|(v, _)| *v == from) else {
                return Err(SceneError::InvalidCell(format!(
                    "no migration from save schema {from} (needed to load {slot:?})"
                )));
            };
            payload = migrate(&payload)?;
            envelope.schema_version += 1;
        }
        let info = info_of(slot, &envelope);
        Ok(LoadedSave { info, payload })
    }

    /// Lists all readable slots, newest first. Damaged files are skipped.
    pub fn list(&self) -> Vec<SlotInfo> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut slots: Vec<SlotInfo> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension()? != "save" {
                    return None;
                }
                let slot = path.file_stem()?.to_str()?.to_owned();
                let envelope = Self::read_envelope(&path).ok()?;
                Some(info_of(&slot, &envelope))
            })
            .collect();
        slots.sort_by(|a, b| {
            b.saved_at_unix
                .cmp(&a.saved_at_unix)
                .then(a.slot.cmp(&b.slot))
        });
        slots
    }

    /// Deletes a slot and its backup. Deleting a missing slot is not an error.
    pub fn delete(&self, slot: &str) -> SceneResult<()> {
        let path = self.slot_path(slot)?;
        for p in [path.clone(), path.with_extension("bak")] {
            match fs::remove_file(&p) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    /// Replaces a slot with its backup (the previous save); the replaced file becomes the new
    /// backup, so calling this twice undoes itself. Errors when there is no backup.
    pub fn restore_backup(&self, slot: &str) -> SceneResult<()> {
        let path = self.slot_path(slot)?;
        let backup = path.with_extension("bak");
        let bytes = fs::read(&backup)?;
        write_replacing(&path, &bytes)?;
        Ok(())
    }
}

fn info_of(slot: &str, envelope: &Envelope) -> SlotInfo {
    SlotInfo {
        slot: slot.to_owned(),
        label: envelope.label.clone(),
        saved_at_unix: envelope.saved_at_unix,
        schema_version: envelope.schema_version,
        game_version: envelope.game_version,
    }
}

/// Writes `bytes` to a temporary file next to `path`, keeps the old file as `.bak`, and
/// renames the temporary file over `path` (which replaces it in one step).
fn write_replacing(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    fs::write(&tmp, bytes)?;
    if path.exists() {
        let _ = fs::copy(path, path.with_extension("bak"));
    }
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}
