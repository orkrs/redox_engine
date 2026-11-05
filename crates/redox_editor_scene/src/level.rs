//! The level open in the editor: either one scene file or one open world folder.
//!
//! Dirty state is computed against a baseline snapshot taken when the level was opened or last
//! saved. EDT-A3 replaces it with the position in the undo stack.

use std::path::{Path, PathBuf};

use redox_ecs::World;
use redox_reflect::{TypeRegistry, WorldSnapshot};
use redox_scene::ofpa;

use crate::{LoadReport, SceneDocument, SceneError, WorldSession};

/// Suffix that marks a scene file.
const SCENE_SUFFIX: &str = ".scene.ron";
/// Manifest file that marks a world folder.
const WORLD_MANIFEST: &str = "world.ron";

/// What the level is made of.
// One value per open level, so the size difference of the variants costs nothing.
#[allow(clippy::large_enum_variant)]
pub enum LevelKind {
    /// One `*.scene.ron` file.
    Scene(SceneDocument),
    /// A world folder with cells and actors.
    World(WorldSession),
}

/// A level open in the editor: its kind and the baseline used for the dirty state.
pub struct Level {
    kind: LevelKind,
    saved: WorldSnapshot,
}

/// Errors of opening and saving a level.
#[derive(Debug, thiserror::Error)]
pub enum LevelError {
    /// The path is neither a scene file nor a world folder.
    #[error("unsupported level at {}", path.display())]
    Unsupported {
        /// The path that was given.
        path: PathBuf,
    },
    /// A world level cannot be written to a new scene file.
    #[error("a world level cannot be saved as a scene file")]
    SaveAsUnsupported,
    /// Scene file error.
    #[error("scene: {0}")]
    Scene(#[from] SceneError),
    /// World folder error.
    #[error("world: {0}")]
    World(#[from] redox_scene::SceneError),
}

impl Level {
    /// A new, empty scene level named `name`. The current world is the baseline.
    pub fn new_scene(name: &str, registry: &TypeRegistry, world: &World) -> Self {
        Self::with_kind(LevelKind::Scene(SceneDocument::new(name)), registry, world)
    }

    /// Opens a scene file (`*.scene.ron`) or a world folder (with `world.ron`).
    ///
    /// A scene's entities are spawned into `world`; a world only attaches its folder, its
    /// cells are streamed later. Any other path gives [`LevelError::Unsupported`].
    pub fn open(
        path: impl AsRef<Path>,
        registry: &TypeRegistry,
        world: &mut World,
    ) -> Result<(Self, LoadReport), LevelError> {
        Self::open_with_assets(path, registry, world, None)
    }

    /// Like [`Self::open`]; a scene also checks its asset handles against `assets_root` and
    /// reports missing files as warnings.
    pub fn open_with_assets(
        path: impl AsRef<Path>,
        registry: &TypeRegistry,
        world: &mut World,
        assets_root: Option<&Path>,
    ) -> Result<(Self, LoadReport), LevelError> {
        let path = path.as_ref();
        if is_scene_file(path) {
            let (document, report) =
                SceneDocument::open_with_assets(path, registry, world, assets_root)?;
            let level = Self::with_kind(LevelKind::Scene(document), registry, world);
            Ok((level, report))
        } else if path.is_dir() && path.join(WORLD_MANIFEST).is_file() {
            let session = WorldSession::open(path)?;
            let level = Self::with_kind(LevelKind::World(session), registry, world);
            Ok((level, LoadReport::default()))
        } else {
            Err(LevelError::Unsupported {
                path: path.to_path_buf(),
            })
        }
    }

    /// Name of the level: the scene name or the world name.
    pub fn title(&self) -> String {
        match &self.kind {
            LevelKind::Scene(document) => document.name.clone(),
            LevelKind::World(session) => session.manifest.name.clone(),
        }
    }

    /// File of a scene level or folder of a world level; `None` for a scene not saved yet.
    pub fn path(&self) -> Option<&Path> {
        match &self.kind {
            LevelKind::Scene(document) => document.path.as_deref(),
            LevelKind::World(session) => Some(session.root.as_path()),
        }
    }

    /// Whether the level is a world folder.
    pub fn is_world(&self) -> bool {
        matches!(self.kind, LevelKind::World(_))
    }

    /// The kind of the level, for panels that need the scene or the world.
    pub fn kind(&self) -> &LevelKind {
        &self.kind
    }

    /// Whether the world differs from the baseline taken at the last open or save.
    pub fn is_dirty(&self, registry: &TypeRegistry, world: &World) -> bool {
        self.saved != canonical_snapshot(registry, world)
    }

    /// Saves to the level's own path. A world level rewrites its manifest; its actors are
    /// written by the paint and actor operations as they happen.
    pub fn save(&mut self, registry: &TypeRegistry, world: &mut World) -> Result<(), LevelError> {
        match &mut self.kind {
            LevelKind::Scene(document) => document.save(registry, world)?,
            LevelKind::World(session) => {
                ofpa::save_manifest(&session.root, &session.manifest)?;
            }
        }
        self.saved = canonical_snapshot(registry, world);
        Ok(())
    }

    /// Saves a scene level to `path` and makes it the level's path. A world level cannot be
    /// saved as a scene file.
    pub fn save_as(
        &mut self,
        path: impl AsRef<Path>,
        registry: &TypeRegistry,
        world: &mut World,
    ) -> Result<(), LevelError> {
        match &mut self.kind {
            LevelKind::Scene(document) => document.save_as(path, registry, world)?,
            LevelKind::World(_) => return Err(LevelError::SaveAsUnsupported),
        }
        self.saved = canonical_snapshot(registry, world);
        Ok(())
    }

    fn with_kind(kind: LevelKind, registry: &TypeRegistry, world: &World) -> Self {
        Self {
            kind,
            saved: canonical_snapshot(registry, world),
        }
    }
}

fn is_scene_file(path: &Path) -> bool {
    path.is_file() && path.to_string_lossy().ends_with(SCENE_SUFFIX)
}

/// The reflected state of the world with entities in a fixed order, so that two worlds with the
/// same content compare equal however their archetypes are laid out.
fn canonical_snapshot(registry: &TypeRegistry, world: &World) -> WorldSnapshot {
    let mut snapshot = registry.snapshot_world(world);
    snapshot
        .entities
        .sort_by_cached_key(|entity| format!("{entity:?}"));
    snapshot
}
