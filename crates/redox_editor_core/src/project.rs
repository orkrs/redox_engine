//! The project manifest `project.ron`: creating, opening and migrating a project, the list of
//! recently opened projects and UTC time stamps for the manifest.

use crate::project_template::{ProjectTemplate, START_SCENE_FILE};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// File name of the project manifest in the project root.
pub const MANIFEST_FILE: &str = "project.ron";
/// Current format version of `project.ron`.
pub const PROJECT_FORMAT_VERSION: u32 = 1;
/// Folder for scenes, relative to the project root.
pub const SCENES_DIR: &str = "scenes";
/// Default assets folder, relative to the project root.
pub const DEFAULT_ASSETS_ROOT: &str = "assets";
/// Editor-private folder in the project root; version control ignores it.
pub const PRIVATE_DIR: &str = ".redox";
/// Suffix of scene files.
pub const SCENE_SUFFIX: &str = ".scene.ron";
/// Recent-projects file, kept next to the editor layout file.
pub const RECENT_FILE: &str = "editor_recent.ron";
/// Largest number of recent projects kept.
pub const RECENT_LIMIT: usize = 10;
/// Version of this editor core, written into new and migrated manifests.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Build profile placeholder. The list stays empty until the build slice (C1) fills it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildProfileStub {
    /// Profile name.
    pub name: String,
}

/// Contents of `project.ron`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectManifest {
    /// Format version of the file.
    pub format_version: u32,
    /// Project name.
    pub name: String,
    /// Version of the editor core that wrote the manifest.
    pub engine_version: String,
    /// Scene opened with the project, relative to the root. Empty when there is none.
    #[serde(default)]
    pub start_scene: String,
    /// Assets folder, relative to the root.
    #[serde(default = "default_assets_root")]
    pub assets_root: String,
    /// Names of the plugins the project asks for.
    #[serde(default)]
    pub plugins: Vec<String>,
    /// Build profiles of the project.
    #[serde(default)]
    pub build_profiles: Vec<BuildProfileStub>,
    /// Creation time, UTC, `YYYY-MM-DDTHH:MM:SSZ`.
    pub created: String,
    /// Time of the last opening, UTC, `YYYY-MM-DDTHH:MM:SSZ`.
    pub last_opened: String,
}

fn default_assets_root() -> String {
    DEFAULT_ASSETS_ROOT.to_string()
}

/// A problem that does not stop a project from opening; the editor shows it in the report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectWarning {
    /// A plugin is listed in the manifest, but no plugin loader exists yet.
    MissingPlugin {
        /// Plugin name from the manifest.
        name: String,
    },
    /// The start scene named in the manifest does not exist.
    MissingStartScene {
        /// Expected location of the scene.
        path: PathBuf,
    },
}

/// Errors of project files.
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    /// File-system error at `path`.
    #[error("io error at {}: {source}", path.display())]
    Io {
        /// File or folder that failed.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// There is no `project.ron` at `path`.
    #[error("no project manifest at {}", path.display())]
    Missing {
        /// Expected manifest location.
        path: PathBuf,
    },
    /// A project already exists at `path`.
    #[error("a project already exists at {}", path.display())]
    AlreadyExists {
        /// Existing manifest location.
        path: PathBuf,
    },
    /// The manifest cannot be parsed.
    #[error("cannot parse {}: {message}", path.display())]
    Parse {
        /// Manifest location.
        path: PathBuf,
        /// Parser message with position.
        message: String,
    },
    /// Writing the manifest failed.
    #[error("cannot write project data: {message}")]
    Serialize {
        /// Serializer message.
        message: String,
    },
    /// The manifest uses a newer format than this editor reads.
    #[error("project format {found} is newer than the supported {supported}")]
    NewerFormat {
        /// Format version in the file.
        found: u32,
        /// Highest version this editor reads.
        supported: u32,
    },
    /// The project was written by a newer engine than this editor.
    #[error("project was written by engine {found}, this editor is {current}")]
    NewerEngine {
        /// Engine version in the manifest.
        found: String,
        /// Version of this editor.
        current: String,
    },
    /// A version string is not `MAJOR.MINOR.PATCH`.
    #[error("invalid version '{0}', expected MAJOR.MINOR.PATCH")]
    InvalidVersion(String),
}

/// An open project: its root folder, manifest and load warnings.
#[derive(Clone, Debug)]
pub struct ProjectContext {
    root: PathBuf,
    manifest: ProjectManifest,
    warnings: Vec<ProjectWarning>,
}

impl ProjectContext {
    /// Creates the folder layout (`scenes/`, `assets/`, `.redox/`), the start scene of
    /// `template` and `project.ron`, and returns the new project.
    pub fn create(
        root: impl AsRef<Path>,
        name: &str,
        template: ProjectTemplate,
    ) -> Result<Self, ProjectError> {
        let root = root.as_ref().to_path_buf();
        let manifest_path = root.join(MANIFEST_FILE);
        if manifest_path.exists() {
            return Err(ProjectError::AlreadyExists {
                path: manifest_path,
            });
        }
        let folders = [
            root.clone(),
            root.join(SCENES_DIR),
            root.join(DEFAULT_ASSETS_ROOT),
            root.join(PRIVATE_DIR),
        ];
        for folder in &folders {
            fs::create_dir_all(folder).map_err(|source| io_error(folder, source))?;
        }
        let private_ignore = root.join(PRIVATE_DIR).join(".gitignore");
        write_text(&private_ignore, "*\n")?;
        let start_scene = format!("{SCENES_DIR}/{START_SCENE_FILE}");
        write_text(&root.join(&start_scene), template.scene_text())?;
        let now = utc_now();
        let manifest = ProjectManifest {
            format_version: PROJECT_FORMAT_VERSION,
            name: name.to_string(),
            engine_version: ENGINE_VERSION.to_string(),
            start_scene,
            assets_root: DEFAULT_ASSETS_ROOT.to_string(),
            plugins: Vec::new(),
            build_profiles: Vec::new(),
            created: now.clone(),
            last_opened: now,
        };
        let context = Self {
            root,
            manifest,
            warnings: Vec::new(),
        };
        context.save_manifest()?;
        Ok(context)
    }

    /// Opens the project under `root`. A newer engine version is refused; an older one is
    /// migrated: `project.ron.bak-v<N>` keeps the first copy of the old file. Missing plugins
    /// and a missing start scene only add warnings.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let root = root.as_ref().to_path_buf();
        let manifest_path = root.join(MANIFEST_FILE);
        let text = fs::read_to_string(&manifest_path).map_err(|source| {
            if source.kind() == ErrorKind::NotFound {
                ProjectError::Missing {
                    path: manifest_path.clone(),
                }
            } else {
                io_error(&manifest_path, source)
            }
        })?;
        let mut manifest: ProjectManifest =
            ron::from_str::<ProjectManifest>(&text).map_err(|err| ProjectError::Parse {
                path: manifest_path.clone(),
                message: err.to_string(),
            })?;
        if manifest.format_version > PROJECT_FORMAT_VERSION {
            return Err(ProjectError::NewerFormat {
                found: manifest.format_version,
                supported: PROJECT_FORMAT_VERSION,
            });
        }
        match compare_engine_version(&manifest.engine_version)? {
            Ordering::Greater => {
                return Err(ProjectError::NewerEngine {
                    found: manifest.engine_version.clone(),
                    current: ENGINE_VERSION.to_string(),
                });
            }
            Ordering::Less => migrate(&manifest_path, &mut manifest)?,
            Ordering::Equal => {}
        }
        let warnings = check_references(&root, &manifest);
        Ok(Self {
            root,
            manifest,
            warnings,
        })
    }

    /// Project root folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Contents of `project.ron`.
    pub fn manifest(&self) -> &ProjectManifest {
        &self.manifest
    }

    /// Warnings found when the project was opened.
    pub fn warnings(&self) -> &[ProjectWarning] {
        &self.warnings
    }

    /// Full path of the start scene, or `None` when the manifest names none.
    pub fn start_scene_path(&self) -> Option<PathBuf> {
        if self.manifest.start_scene.is_empty() {
            None
        } else {
            Some(self.root.join(&self.manifest.start_scene))
        }
    }

    /// Writes `project.ron` with the current manifest.
    pub fn save_manifest(&self) -> Result<(), ProjectError> {
        write_manifest(&self.root.join(MANIFEST_FILE), &self.manifest)
    }

    /// Sets `last_opened` to the current time and writes the manifest.
    pub fn mark_opened(&mut self) -> Result<(), ProjectError> {
        self.manifest.last_opened = utc_now();
        self.save_manifest()
    }

    /// Every `*.scene.ron` file under the root, sorted; the private folder is skipped.
    pub fn scenes(&self) -> Result<Vec<PathBuf>, ProjectError> {
        let mut found = Vec::new();
        collect_scenes(&self.root, &mut found)?;
        found.sort();
        Ok(found)
    }
}

/// Recently opened project roots, most recent first, without duplicates.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentProjects {
    entries: Vec<String>,
}

impl RecentProjects {
    /// Reads the list from `path`; a missing file gives an empty list.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let path = path.as_ref();
        match fs::read_to_string(path) {
            Ok(text) => ron::from_str::<Self>(&text).map_err(|err| ProjectError::Parse {
                path: path.to_path_buf(),
                message: err.to_string(),
            }),
            Err(source) if source.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(io_error(path, source)),
        }
    }

    /// Writes the list to `path`.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), ProjectError> {
        write_text(path.as_ref(), &to_ron(self)?)
    }

    /// Project roots, most recent first.
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// Moves `root` to the front, dropping an older copy and keeping at most [`RECENT_LIMIT`].
    pub fn push(&mut self, root: &Path) {
        let name = root.display().to_string();
        self.entries.retain(|known| *known != name);
        self.entries.insert(0, name);
        self.entries.truncate(RECENT_LIMIT);
    }
}

/// Formats Unix seconds as `YYYY-MM-DDTHH:MM:SSZ` (UTC, proleptic Gregorian calendar).
pub fn format_utc(unix_seconds: u64) -> String {
    const SECONDS_PER_DAY: u64 = 86_400;
    let days = (unix_seconds / SECONDS_PER_DAY) as i64;
    let rest = unix_seconds % SECONDS_PER_DAY;
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (rest / 3600, (rest % 3600) / 60, rest % 60);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// The current time as a UTC string. A clock before 1970 counts as the epoch.
pub fn utc_now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    format_utc(seconds)
}

/// Converts days since 1970-01-01 to (year, month, day). Howard Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month as u32, day as u32)
}

/// Compares `found` with [`ENGINE_VERSION`].
fn compare_engine_version(found: &str) -> Result<Ordering, ProjectError> {
    let found_parts =
        parse_version(found).ok_or_else(|| ProjectError::InvalidVersion(found.to_string()))?;
    let current = parse_version(ENGINE_VERSION)
        .ok_or_else(|| ProjectError::InvalidVersion(ENGINE_VERSION.to_string()))?;
    Ok(found_parts.cmp(&current))
}

fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let mut parts = text.split('.').map(|part| part.parse::<u32>().ok());
    let major = parts.next()??;
    let minor = parts.next()??;
    let patch = parts.next()??;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Keeps the first copy of the old manifest as `project.ron.bak-v<N>` and rewrites the manifest
/// with the current engine version. An existing backup is never overwritten.
fn migrate(manifest_path: &Path, manifest: &mut ProjectManifest) -> Result<(), ProjectError> {
    let backup = PathBuf::from(format!(
        "{}.bak-v{}",
        manifest_path.display(),
        manifest.format_version
    ));
    if !backup.exists() {
        fs::copy(manifest_path, &backup).map_err(|source| io_error(&backup, source))?;
    }
    manifest.engine_version = ENGINE_VERSION.to_string();
    write_manifest(manifest_path, manifest)
}

fn check_references(root: &Path, manifest: &ProjectManifest) -> Vec<ProjectWarning> {
    let mut warnings: Vec<ProjectWarning> = manifest
        .plugins
        .iter()
        .map(|name| ProjectWarning::MissingPlugin { name: name.clone() })
        .collect();
    if !manifest.start_scene.is_empty() {
        let path = root.join(&manifest.start_scene);
        if !path.is_file() {
            warnings.push(ProjectWarning::MissingStartScene { path });
        }
    }
    warnings
}

fn collect_scenes(folder: &Path, found: &mut Vec<PathBuf>) -> Result<(), ProjectError> {
    let entries = fs::read_dir(folder).map_err(|source| io_error(folder, source))?;
    for entry in entries {
        let entry = entry.map_err(|source| io_error(folder, source))?;
        let path = entry.path();
        let is_private = entry.file_name().to_string_lossy() == PRIVATE_DIR;
        if path.is_dir() {
            if !is_private {
                collect_scenes(&path, found)?;
            }
        } else if path.to_string_lossy().ends_with(SCENE_SUFFIX) {
            found.push(path);
        }
    }
    Ok(())
}

fn write_manifest(path: &Path, manifest: &ProjectManifest) -> Result<(), ProjectError> {
    write_text(path, &to_ron(manifest)?)
}

fn to_ron<T: Serialize>(value: &T) -> Result<String, ProjectError> {
    ron::ser::to_string_pretty(value, ron::ser::PrettyConfig::default()).map_err(|err| {
        ProjectError::Serialize {
            message: err.to_string(),
        }
    })
}

/// Writes `text` to a temporary file next to `path` and renames it over `path`.
fn write_text(path: &Path, text: &str) -> Result<(), ProjectError> {
    redox_scene::write_atomic(path, text.as_bytes()).map_err(|source| io_error(path, source))
}

fn io_error(path: &Path, source: std::io::Error) -> ProjectError {
    ProjectError::Io {
        path: path.to_path_buf(),
        source,
    }
}
