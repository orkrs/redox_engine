//! The scene document: opening and saving `*.scene.ron` files with stable entity identity.
//!
//! Save writes entities sorted by guid, so saving an unchanged world gives the same bytes however
//! the world orders its archetypes (small diffs in version control). Load spawns entities in file order and links parents in a second pass. Unknown
//! components are kept and written back, and every problem found while loading is reported
//! instead of being dropped silently.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use redox_ecs::{Entity, Parent, World, set_parent};
use redox_editor_core::editor_only::EditorOnly;
use redox_reflect::{Reflect, ReflectValue, TypeRegistry};
use redox_scene::{AssetRefs, EntityGuid, ParentRef, write_atomic};

use crate::format::{SCENE_FORMAT_VERSION, SceneEntity, SceneFile, migrate};
use crate::guid_index::{GuidIndex, assign_missing_guids};

/// Handle component types whose asset path is checked against the assets root on load.
const HANDLE_TYPES: [&str; 2] = ["MeshHandle", "MaterialHandle"];

/// Errors of opening and saving scene files.
#[derive(Debug, thiserror::Error)]
pub enum SceneError {
    /// The file could not be read or written.
    #[error("cannot access {}: {source}", .path.display())]
    Io {
        /// File that failed.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// The file is not valid RON for the scene format.
    #[error("cannot parse {} at line {line}, column {column}: {message}", .path.display())]
    Parse {
        /// File that failed.
        path: PathBuf,
        /// Line of the error, starting at 1.
        line: usize,
        /// Column of the error, starting at 1.
        column: usize,
        /// Parser message.
        message: String,
    },
    /// The file was written by a newer editor than this one.
    #[error("scene format {found} is newer than the supported format {supported}")]
    NewerFormat {
        /// Version found in the file.
        found: u32,
        /// Highest version this build reads.
        supported: u32,
    },
    /// The content is structurally wrong, for example two entities share a guid.
    #[error("invalid scene: {reason}")]
    Invalid {
        /// What is wrong.
        reason: String,
    },
    /// The world could not be written as RON.
    #[error("cannot serialize the scene: {0}")]
    Serialize(String),
}

/// A problem found while loading. The load itself succeeds; the editor shows these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadWarning {
    /// The component type is not registered. Its value is kept and written back on save.
    UnknownComponent {
        /// Entity that carries it.
        entity: EntityGuid,
        /// Type name as written in the file.
        type_name: String,
    },
    /// The parent is not in the file. The entity was loaded without a parent.
    MissingParent {
        /// Child entity.
        entity: EntityGuid,
        /// Guid the file names as parent.
        parent_guid: EntityGuid,
    },
    /// A handle refers to a file that does not exist under the assets root.
    MissingAsset {
        /// Entity that carries the handle.
        entity: EntityGuid,
        /// Handle type name.
        type_name: String,
        /// Path written in the file.
        path: String,
    },
    /// The entity had no guid. A new one was assigned.
    MissingGuid {
        /// Index of the entity in the file.
        entity: usize,
    },
}

/// Everything a load reported, in the order found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadReport {
    /// Warnings for the user.
    pub warnings: Vec<LoadWarning>,
}

/// An open `*.scene.ron` document: its file, name and the unknown components it carries.
#[derive(Debug, Clone)]
pub struct SceneDocument {
    /// File the document is saved to. `None` until it is saved or opened.
    pub path: Option<PathBuf>,
    /// Display name of the scene.
    pub name: String,
    /// Format version of the document. Saving always writes [`SCENE_FORMAT_VERSION`].
    pub format_version: u32,
    /// Version found on disk when it was older than the current one. The next save keeps a
    /// copy of the old file first.
    migrated_from: Option<u32>,
    /// Unknown components per entity, as read from the file.
    unknown: HashMap<EntityGuid, Vec<ReflectValue>>,
}

impl SceneDocument {
    /// Creates an empty document that has no file yet.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            path: None,
            name: name.into(),
            format_version: SCENE_FORMAT_VERSION,
            migrated_from: None,
            unknown: HashMap::new(),
        }
    }

    /// Opens a scene file into `world` without checking asset references.
    ///
    /// Returns the document and the load report. The file is only read; nothing is written.
    pub fn open(
        path: impl AsRef<Path>,
        registry: &TypeRegistry,
        world: &mut World,
    ) -> Result<(Self, LoadReport), SceneError> {
        Self::open_with_assets(path, registry, world, None)
    }

    /// Opens a scene file into `world`. Handle paths are checked against `assets_root` when it
    /// is given, and missing files are reported as [`LoadWarning::MissingAsset`].
    pub fn open_with_assets(
        path: impl AsRef<Path>,
        registry: &TypeRegistry,
        world: &mut World,
        assets_root: Option<&Path>,
    ) -> Result<(Self, LoadReport), SceneError> {
        let path = path.as_ref();
        let text = fs::read_to_string(path).map_err(|source| io_error(path, source))?;
        let mut file: SceneFile = ron::from_str(&text).map_err(|err| parse_error(path, &err))?;
        let found = file.format_version;
        if found > SCENE_FORMAT_VERSION {
            return Err(SceneError::NewerFormat {
                found,
                supported: SCENE_FORMAT_VERSION,
            });
        }
        migrate(&mut file, found);

        let loader = Loader {
            registry,
            assets_root,
            index: GuidIndex::new(),
            report: LoadReport::default(),
            unknown: HashMap::new(),
        };
        let (report, unknown) = loader.run(&file, world)?;
        let document = Self {
            path: Some(path.to_path_buf()),
            name: file.name,
            format_version: SCENE_FORMAT_VERSION,
            migrated_from: (found < SCENE_FORMAT_VERSION).then_some(found),
            unknown,
        };
        Ok((document, report))
    }

    /// Saves to the document's file. Fails when the document has no file yet; use
    /// [`Self::save_as`] then.
    ///
    /// Entities without a guid get one first, so the world changes.
    pub fn save(&mut self, registry: &TypeRegistry, world: &mut World) -> Result<(), SceneError> {
        let path = self.path.clone().ok_or_else(|| SceneError::Invalid {
            reason: "the scene has no file yet; use save_as".into(),
        })?;
        self.write_to(&path, registry, world)
    }

    /// Saves to `path` and makes it the document's file. The previous file is untouched if the
    /// write fails.
    pub fn save_as(
        &mut self,
        path: impl AsRef<Path>,
        registry: &TypeRegistry,
        world: &mut World,
    ) -> Result<(), SceneError> {
        let path = path.as_ref();
        self.write_to(path, registry, world)?;
        self.path = Some(path.to_path_buf());
        Ok(())
    }

    /// The bytes a save would write, without touching any file. Entities without a guid get one
    /// first, so the world changes exactly as it does on save.
    pub fn render_bytes(
        &self,
        registry: &TypeRegistry,
        world: &mut World,
    ) -> Result<Vec<u8>, SceneError> {
        assign_missing_guids(world);
        let file = self.build_file(registry, world);
        ron::ser::to_string_pretty(&file, ron::ser::PrettyConfig::default())
            .map(String::into_bytes)
            .map_err(|err| SceneError::Serialize(err.to_string()))
    }

    fn write_to(
        &mut self,
        path: &Path,
        registry: &TypeRegistry,
        world: &mut World,
    ) -> Result<(), SceneError> {
        assign_missing_guids(world);
        let file = self.build_file(registry, world);
        let text = ron::ser::to_string_pretty(&file, ron::ser::PrettyConfig::default())
            .map_err(|err| SceneError::Serialize(err.to_string()))?;
        self.backup_if_migrated(path)?;
        write_atomic(path, text.as_bytes()).map_err(|source| io_error(path, source))?;
        self.format_version = SCENE_FORMAT_VERSION;
        self.migrated_from = None;
        Ok(())
    }

    /// Keeps a copy of the old file before it is replaced by a newer format.
    fn backup_if_migrated(&self, path: &Path) -> Result<(), SceneError> {
        if let Some(from) = self.migrated_from
            && path.exists()
        {
            let backup = backup_path(path, from)?;
            fs::copy(path, &backup).map_err(|source| io_error(&backup, source))?;
        }
        Ok(())
    }

    fn build_file(&self, registry: &TypeRegistry, world: &World) -> SceneFile {
        let mut index = GuidIndex::new();
        index.rebuild(world);
        let mut entities: Vec<SceneEntity> = world
            .all_entities()
            .filter_map(|entity| self.scene_entity(registry, world, &index, entity))
            .collect();
        entities.sort_by_key(|entity| entity.guid);
        SceneFile {
            format_version: SCENE_FORMAT_VERSION,
            name: self.name.clone(),
            entities,
        }
    }

    fn scene_entity(
        &self,
        registry: &TypeRegistry,
        world: &World,
        index: &GuidIndex,
        entity: Entity,
    ) -> Option<SceneEntity> {
        if world.get_component::<EditorOnly>(entity).is_some() {
            return None;
        }
        let guid = world.get_component::<EntityGuid>(entity).copied()?;
        let mut components = registry.snapshot_entity(world, entity);
        apply_saved_refs(&mut components, world.get_component::<AssetRefs>(entity));
        if let Some(Parent(parent)) = world.get_component::<Parent>(entity).copied()
            && let Some(parent_guid) = index.guid_of(parent)
        {
            components.push(ParentRef(parent_guid).to_value());
        }
        Some(SceneEntity {
            guid: Some(guid),
            components,
            unknown_components: self.unknown.get(&guid).cloned().unwrap_or_default(),
        })
    }
}

/// A child whose parent link is applied after every entity exists.
struct PendingParent {
    entity: Entity,
    guid: EntityGuid,
    parent: EntityGuid,
}

/// State of one load: the registry, the optional assets root and what has been collected.
struct Loader<'a> {
    registry: &'a TypeRegistry,
    assets_root: Option<&'a Path>,
    index: GuidIndex,
    report: LoadReport,
    unknown: HashMap<EntityGuid, Vec<ReflectValue>>,
}

impl Loader<'_> {
    /// Spawns every entity of `file`, then links the parents. Returns the report and the
    /// unknown components per entity.
    fn run(
        mut self,
        file: &SceneFile,
        world: &mut World,
    ) -> Result<(LoadReport, HashMap<EntityGuid, Vec<ReflectValue>>), SceneError> {
        let mut pending = Vec::new();
        for (position, entry) in file.entities.iter().enumerate() {
            if let Some(link) = self.spawn_entity(entry, position, world)? {
                pending.push(link);
            }
        }
        for link in pending {
            self.link(link, world);
        }
        Ok((self.report, self.unknown))
    }

    fn spawn_entity(
        &mut self,
        entry: &SceneEntity,
        position: usize,
        world: &mut World,
    ) -> Result<Option<PendingParent>, SceneError> {
        let guid = self.guid_of_entry(entry, position);
        if self.index.get(guid).is_some() {
            return Err(SceneError::Invalid {
                reason: format!("two entities share the guid {guid}"),
            });
        }
        let entity = world.spawn(); // undo: init
        world.add_component(entity, guid); // undo: init
        self.index.insert(guid, entity);

        let mut parent = None;
        for value in entry.components.iter().chain(&entry.unknown_components) {
            if let Some(found) = self.load_value(value, guid, entity, world)? {
                parent = Some(found);
            }
        }
        Ok(parent.map(|parent| PendingParent {
            entity,
            guid,
            parent,
        }))
    }

    /// Applies one reflected value. Returns the parent guid for a `ParentRef` value.
    fn load_value(
        &mut self,
        value: &ReflectValue,
        guid: EntityGuid,
        entity: Entity,
        world: &mut World,
    ) -> Result<Option<EntityGuid>, SceneError> {
        let type_name = struct_type_name(value)?;
        if type_name == <ParentRef as Reflect>::type_name() {
            return match ParentRef::from_value(value) {
                Some(parent) => Ok(Some(parent.0)),
                None => Err(SceneError::Invalid {
                    reason: format!("entity {guid} has a broken parent reference"),
                }),
            };
        }
        if self.registry.schema(type_name).is_none() {
            self.report.warnings.push(LoadWarning::UnknownComponent {
                entity: guid,
                type_name: type_name.to_owned(),
            });
            self.unknown.entry(guid).or_default().push(value.clone());
            return Ok(None);
        }
        self.check_asset(type_name, value, guid);
        self.registry
            .insert_component(world, entity, value) // undo: init
            .map_err(|err| SceneError::Invalid {
                reason: err.to_string(),
            })?;
        if HANDLE_TYPES.contains(&type_name) {
            store_ref(world, entity, type_name, value);
        }
        Ok(None)
    }

    fn check_asset(&mut self, type_name: &str, value: &ReflectValue, guid: EntityGuid) {
        let Some(root) = self.assets_root else {
            return;
        };
        if !HANDLE_TYPES.contains(&type_name) {
            return;
        }
        if let Some(Some(path)) = value.as_handle_path()
            && !root.join(&path).exists()
        {
            self.report.warnings.push(LoadWarning::MissingAsset {
                entity: guid,
                type_name: type_name.to_owned(),
                path,
            });
        }
    }

    fn guid_of_entry(&mut self, entry: &SceneEntity, position: usize) -> EntityGuid {
        match entry.guid {
            Some(guid) => guid,
            None => {
                self.report
                    .warnings
                    .push(LoadWarning::MissingGuid { entity: position });
                EntityGuid::new_random()
            }
        }
    }

    fn link(&mut self, link: PendingParent, world: &mut World) {
        match self.index.get(link.parent) {
            Some(parent) => set_parent(world, link.entity, parent),
            None => self.report.warnings.push(LoadWarning::MissingParent {
                entity: link.guid,
                parent_guid: link.parent,
            }),
        }
    }
}

/// Rewrites the path hint of every stored reference whose guid `resolve` knows.
///
/// Call it after an asset is renamed or moved so the open scene saves the new path. Returns the
/// number of hints that changed.
pub fn refresh_ref_paths(world: &mut World, resolve: impl Fn(&str) -> Option<String>) -> usize {
    let mut changed = 0;
    let entities: Vec<Entity> = world.all_entities().collect();
    for entity in entities {
        let Some(refs) = world.get_component_mut::<AssetRefs>(entity) else {
            continue;
        };
        for item in &mut refs.0 {
            let Some(path) = item.guid.as_deref().and_then(&resolve) else {
                continue;
            };
            if item.path.as_deref() != Some(path.as_str()) {
                item.path = Some(path);
                changed += 1;
            }
        }
    }
    changed
}

/// Keeps the guid and path of a handle value in the entity's [`AssetRefs`].
fn store_ref(world: &mut World, entity: Entity, type_name: &str, value: &ReflectValue) {
    let guid = value
        .field("guid")
        .and_then(ReflectValue::as_string)
        .map(str::to_owned);
    let path = value.as_handle_path().flatten();
    if world.get_component::<AssetRefs>(entity).is_none() {
        world.add_component(entity, AssetRefs::default()); // undo: init
    }
    if let Some(refs) = world.get_component_mut::<AssetRefs>(entity) {
        refs.set(type_name, guid, path);
    }
}

/// Puts the stored guid and path back into the handle values and drops the `AssetRefs` value
/// itself, which is never written to a scene file.
fn apply_saved_refs(components: &mut Vec<ReflectValue>, refs: Option<&AssetRefs>) {
    components.retain(|value| struct_type_name(value).is_ok_and(|name| name != "AssetRefs"));
    let Some(refs) = refs else {
        return;
    };
    for value in components.iter_mut() {
        let Ok(name) = struct_type_name(value).map(str::to_owned) else {
            continue;
        };
        if !HANDLE_TYPES.contains(&name.as_str()) {
            continue;
        }
        if let Some(saved) = refs.get(&name) {
            *value = ReflectValue::asset_ref(name, saved.guid.clone(), saved.path.clone());
        }
    }
}

fn struct_type_name(value: &ReflectValue) -> Result<&str, SceneError> {
    match value {
        ReflectValue::Struct { type_name, .. } => Ok(type_name.as_str()),
        _ => Err(SceneError::Invalid {
            reason: "a component value is not a struct".into(),
        }),
    }
}

fn backup_path(path: &Path, from: u32) -> Result<PathBuf, SceneError> {
    let Some(name) = path.file_name() else {
        return Err(SceneError::Invalid {
            reason: format!("{} has no file name", path.display()),
        });
    };
    Ok(path.with_file_name(format!("{}.bak-v{from}", name.to_string_lossy())))
}

fn io_error(path: &Path, source: std::io::Error) -> SceneError {
    SceneError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn parse_error(path: &Path, err: &ron::error::SpannedError) -> SceneError {
    SceneError::Parse {
        path: path.to_path_buf(),
        line: err.span.start.line,
        column: err.span.start.col,
        message: err.code.to_string(),
    }
}
