//! The editor test handle `EditorHarness` (EDT-A7).
//!
//! The handle drives the editor without a window, GPU or network. It calls the same commands
//! and services as the app (`CommandRegistry::run`, `Level`, `ProjectContext`,
//! `AutosaveService`, `AssetService`, `PieSession`) and holds no product logic of its own.
//! Time is replaced by [`TestClock`].
//!
//! Every call into the slices EDT-A2..A6 lives in this file only. Those slices are not in the
//! worktree yet; the signatures used here are listed in
//! `docs/engineering/notes/EDT-A7-report.md`, section "Предположения об API".

#![allow(dead_code)]

pub mod build;
pub mod clock;
pub mod temp;

#[allow(unused_imports)]
pub use build::{
    AssetRef, SceneEntitySpec, copy_dir, corrupt, make_glb, make_png, make_scene,
    write_asset_with_meta,
};
pub use clock::TestClock;
pub use temp::{TempDir, list_files};

use std::path::{Path, PathBuf};

use redox_asset::guid::AssetStatus;
use redox_ecs::hierarchy::{Parent, set_parent};
use redox_ecs::{Entity, World};
use redox_editor_assets::{AssetService, DeleteOutcome};
use redox_editor_core::EditorContext;
use redox_editor_core::EditorLog;
use redox_editor_core::autosave::{AutosavePolicy, AutosaveService, AutosaveSource, WriteMode};
use redox_editor_core::editor_only::{EditorOnly, editor_only_entities};
use redox_editor_core::project::{ProjectContext, RecentProjects};
use redox_editor_core::project_template::ProjectTemplate;
use redox_editor_core::recovery::{
    RecoveryCheck, RecoveryChoice, RecoveryOffer, RecoveryQuery, apply_recovery, check_recovery,
};
use redox_editor_core::settings::EditorSettings;
use redox_editor_core::undo::{UndoCommand, UndoStack};
use redox_editor_debugger::PieSession;
use redox_editor_scene::guid_index::GuidIndex;
use redox_editor_scene::{Level, LoadReport, SceneDocument, register_scene_components};
use redox_math::{Transform, Vec3};
use redox_reflect::{Reflect, ReflectValue, TypeRegistry, WorldSnapshot};
use redox_scene::Name;
use redox_scene::components::EntityGuid;

/// Attempts made while waiting for the import queue to become idle.
const IMPORT_WAIT_ROUNDS: usize = 200_000;
/// Undo steps the harness keeps; large enough for the stress scenario.
const HISTORY_LIMIT: usize = 100_000;
/// Name of the folder inside a project where the editor keeps its service files.
const SERVICE_DIR: &str = ".redox";

/// GUID of `assets/textures/checker.png` in the sample projects (hex form in files).
pub const CHECKER_GUID: u128 = 0x0000_0000_0000_0000_0000_0000_0000_1001;
/// GUID of `assets/models/triangle.glb` in the sample projects.
pub const TRIANGLE_GUID: u128 = 0x0000_0000_0000_0000_0000_0000_0000_1002;

/// Result of a delete request for an asset.
#[derive(Debug, PartialEq, Eq)]
pub enum Deletion {
    /// The file and its sidecar were removed.
    Deleted,
    /// Nothing was removed; these scene paths still reference the asset.
    Blocked(Vec<String>),
}

/// Counts of the asset report produced when a project opens.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AssetCounts {
    /// Sidecars created during the scan.
    pub created: usize,
    /// Sidecars regenerated during the scan.
    pub regenerated: usize,
    /// Damaged sidecars.
    pub damaged: usize,
    /// References to assets that do not exist.
    pub missing_refs: usize,
}

/// What the editor did with a damaged layout file.
#[derive(Debug, PartialEq, Eq)]
pub struct LayoutOutcome {
    /// The default layout is used.
    pub used_default: bool,
    /// The damaged file was renamed to `editor_layout.ron.broken`.
    pub broken_renamed: bool,
}

/// The editor without a window. See the module documentation.
pub struct EditorHarness {
    pub ctx: EditorContext,
    pub project: Option<ProjectContext>,
    pub level: Option<Level>,
    pub assets: Option<AssetService>,
    pub autosave: Option<AutosaveService>,
    pub clock: TestClock,
    pub root: TempDir,
    pub pie: PieSession,
    pub recovery: Option<RecoveryOffer>,
    pub load_report: Option<LoadReport>,
    /// File chosen by [`Self::new_scene`] for the first save.
    pub pending_path: Option<PathBuf>,
    pub asset_report: Option<redox_editor_assets::AssetReport>,
}

impl EditorHarness {
    /// A fresh editor on a fresh temporary directory.
    pub fn new() -> Self {
        Self::on_root(TempDir::new("harness"), TestClock::new())
    }

    fn on_root(root: TempDir, clock: TestClock) -> Self {
        let mut registry = TypeRegistry::new();
        register_scene_components(&mut registry);
        let ctx = EditorContext::new(registry);
        *ctx.undo.lock() = UndoStack::new(HISTORY_LIMIT);
        Self {
            ctx,
            project: None,
            level: None,
            assets: None,
            autosave: None,
            clock,
            root,
            pie: PieSession::new(),
            recovery: None,
            load_report: None,
            pending_path: None,
            asset_report: None,
        }
    }

    /// Closes the editor the normal way and starts a new one on the same files: memory is
    /// reset, the disk is kept.
    pub fn restart(mut self) -> Self {
        let dirty = self.level_is_dirty();
        if let Some(service) = self.autosave.as_mut() {
            let _ = service.finish_session(dirty);
        }
        let Self { root, clock, .. } = self;
        Self::on_root(root, clock)
    }

    /// Starts a new editor on the same files without the normal close: the session lock and
    /// the autosaves stay on disk, as after a crash.
    pub fn crash(self) -> Self {
        let Self { root, clock, .. } = self;
        Self::on_root(root, clock)
    }

    fn level_is_dirty(&self) -> bool {
        let world = self.ctx.world.lock();
        let registry = self.ctx.registry.lock();
        self.level
            .as_ref()
            .is_some_and(|level| level.is_dirty(&registry, &world))
    }

    /// Creates the autosave service of the project and looks for a recovery offer for its start
    /// scene. The session lock is taken later, when a level is open and no offer is pending.
    fn start_autosave(&mut self, dir: &Path) {
        let log = EditorLog::new();
        let policy = self.autosave_policy();
        self.autosave = Some(AutosaveService::new(
            dir,
            policy,
            Box::new(self.clock.clone()),
            log.clone(),
            WriteMode::Synchronous,
        ));
        self.recovery = None;
        let Some(start) = self
            .project
            .as_ref()
            .and_then(ProjectContext::start_scene_path)
        else {
            return;
        };
        let mut scratch = World::new();
        let title = {
            let registry = self.ctx.registry.lock();
            match Level::open(&start, &registry, &mut scratch) {
                Ok((level, _)) => level.title(),
                Err(_) => return,
            }
        };
        let query = RecoveryQuery {
            project_root: dir,
            level: &title,
            level_path: Some(&start),
            now: self.clock.now_secs(),
            interval_secs: policy.interval_secs,
        };
        let is_valid = |bytes: &[u8]| {
            std::str::from_utf8(bytes)
                .ok()
                .is_some_and(|text| text.trim_start().starts_with('('))
        };
        if let RecoveryCheck::Offer(offer) = check_recovery(&query, &is_valid, &log) {
            self.recovery = Some(offer);
        }
    }

    /// Takes the session lock for the open level unless a recovery question is still open.
    fn begin_autosave_session(&mut self) {
        if self.recovery.is_some() {
            return;
        }
        let (Some(level), Some(service)) = (self.level.as_ref(), self.autosave.as_mut()) else {
            return;
        };
        if service.is_session_active() {
            return;
        }
        let source = HarnessSource {
            ctx: &self.ctx,
            level,
        };
        let _ = service.begin_session(&source);
    }

    /// The temporary root of this editor.
    pub fn root(&self) -> &Path {
        self.root.path()
    }

    /// Where the editor keeps `editor_layout.ron`, `editor_recent.ron` and `editor_settings.ron`.
    pub fn config_dir(&self) -> PathBuf {
        self.root().join("config")
    }

    /// Root of the open project. Panics when no project is open (a test bug).
    pub fn project_dir(&self) -> PathBuf {
        self.project
            .as_ref()
            .expect("a project is open")
            .root()
            .to_path_buf()
    }

    /// Creates a project from `template` under the root and opens it.
    pub fn new_project(&mut self, name: &str, template: &str) -> Result<PathBuf, String> {
        let dir = self.root().join("projects").join(name);
        let template = match template {
            "3d_starter" => ProjectTemplate::ThreeDStarter,
            _ => ProjectTemplate::Empty,
        };
        ProjectContext::create(&dir, name, template).map_err(|e| format!("{e:?}"))?;
        self.open_project(&dir)?;
        Ok(dir)
    }

    /// Opens the project in `dir`. Adds it to the recent list and looks for a recovery offer.
    pub fn open_project(&mut self, dir: &Path) -> Result<(), String> {
        let project = ProjectContext::open(dir).map_err(|e| format!("{e:?}"))?;
        let assets = AssetService::open(dir).map_err(|e| format!("{e:?}"))?;
        let recent_file = self.config_dir().join("editor_recent.ron");
        std::fs::create_dir_all(self.config_dir()).map_err(|e| e.to_string())?;
        let mut recent = RecentProjects::load(&recent_file).unwrap_or_default();
        recent.push(dir);
        recent.save(&recent_file).map_err(|e| format!("{e:?}"))?;
        self.asset_report = Some(assets.report().clone());
        self.project = Some(project);
        self.assets = Some(assets);
        self.level = None;
        self.load_report = None;
        self.start_autosave(dir);
        Ok(())
    }

    /// Copies the committed sample `tests/data/<name>` into the root, writes its binary assets
    /// with sidecars, and returns the project directory.
    pub fn install_sample(&self, name: &str) -> PathBuf {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(name);
        let dir = self.root().join("projects").join(name);
        copy_dir(&source, &dir);
        if name == "sample_project" {
            write_asset_with_meta(
                &dir.join("assets/textures/checker.png"),
                &make_png(16, 16),
                CHECKER_GUID,
            );
            write_asset_with_meta(
                &dir.join("assets/models/triangle.glb"),
                &make_glb([[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
                TRIANGLE_GUID,
            );
        } else {
            std::fs::create_dir_all(dir.join("assets/textures"))
                .expect("the asset directory is created");
            std::fs::write(dir.join("assets/textures/damaged.png"), make_png(8, 8))
                .expect("the asset is written");
        }
        dir
    }

    /// The start scene of the open project, relative to its root.
    pub fn start_scene(&self) -> Option<String> {
        self.project
            .as_ref()
            .map(|project| project.manifest().start_scene.clone())
    }

    /// Opens the start scene of the open project.
    pub fn open_start_scene(&mut self) -> Result<(), String> {
        let start = self.start_scene().ok_or("no project is open")?;
        self.open_scene(&start)
    }

    /// Opens a scene file given relative to the project root. Replaces the edited world, the
    /// selection and the undo history.
    pub fn open_scene(&mut self, relative: &str) -> Result<(), String> {
        let path = self.project_dir().join(relative);
        *self.ctx.world.lock() = World::new();
        *self.ctx.undo.lock() = UndoStack::new(HISTORY_LIMIT);
        self.ctx.selection.lock().clear();
        let (level, report) = {
            let mut world = self.ctx.world.lock();
            let registry = self.ctx.registry.lock();
            let assets_root = self.project_dir();
            Level::open_with_assets(&path, &registry, &mut world, Some(&assets_root))
                .map_err(|e| format!("{e:?}"))?
        };
        self.load_report = Some(report);
        self.level = Some(level);
        self.begin_autosave_session();
        Ok(())
    }

    /// Creates an empty scene at `relative` and makes it the open level.
    pub fn new_scene(&mut self, relative: &str) -> Result<(), String> {
        let path = self.project_dir().join(relative);
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("scene")
            .to_owned();
        *self.ctx.world.lock() = World::new();
        *self.ctx.undo.lock() = UndoStack::new(HISTORY_LIMIT);
        let level = {
            let registry = self.ctx.registry.lock();
            let world = self.ctx.world.lock();
            Level::new_scene(&name, &registry, &world)
        };
        self.pending_path = Some(path);
        self.load_report = None;
        self.level = Some(level);
        self.begin_autosave_session();
        Ok(())
    }

    /// Saves the open level to its file. Errors are returned, not only logged.
    pub fn save(&mut self) -> Result<(), String> {
        let level = self.level.as_mut().ok_or("no level is open")?;
        let mut world = self.ctx.world.lock();
        let registry = self.ctx.registry.lock();
        if level.path().is_none() {
            let path = self.pending_path.clone().ok_or("the scene has no file")?;
            return level
                .save_as(&path, &registry, &mut world)
                .map_err(|e| format!("{e:?}"));
        }
        level
            .save(&registry, &mut world)
            .map_err(|e| format!("{e:?}"))
    }

    /// Whether the open level has unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.level_is_dirty()
    }

    /// Recent project directories, most recent first.
    pub fn recent_projects(&self) -> Vec<PathBuf> {
        RecentProjects::load(self.config_dir().join("editor_recent.ron"))
            .map(|recent| recent.entries().iter().map(PathBuf::from).collect())
            .unwrap_or_default()
    }

    /// Warnings of the last project open, as variant names (for example `MissingPlugin`).
    pub fn project_warning_tags(&self) -> Vec<String> {
        self.project
            .as_ref()
            .map(|project| project.warnings().iter().map(variant_name).collect())
            .unwrap_or_default()
    }

    /// Warnings of the last level open, as variant names.
    pub fn load_warning_tags(&self) -> Vec<String> {
        self.load_report
            .as_ref()
            .map(|report| report.warnings.iter().map(variant_name).collect())
            .unwrap_or_default()
    }

    /// Counts of the asset report of the last project open.
    pub fn asset_counts(&self) -> AssetCounts {
        match self.asset_report.as_ref() {
            Some(report) => AssetCounts {
                created: report.created.len(),
                regenerated: report.regenerated.len(),
                damaged: report.damaged.len(),
                missing_refs: report.missing_refs.len(),
            },
            None => AssetCounts::default(),
        }
    }

    /// Runs the command `id` from the command registry. `false` for an unknown id.
    pub fn run(&self, id: &str) -> bool {
        self.ctx.commands.run(&self.ctx, id)
    }

    /// Snapshot of every registered component of the world.
    pub fn snapshot(&self) -> WorldSnapshot {
        let world = self.ctx.world.lock();
        let registry = self.ctx.registry.lock();
        let mut snapshot = registry.snapshot_world(&world);
        // The order of entities depends on archetype layout; compare as a set.
        snapshot
            .entities
            .sort_by_key(|entity| format!("{entity:?}"));
        snapshot
    }

    /// Number of entities in the world.
    pub fn entity_count(&self) -> usize {
        self.ctx.world.lock().all_entities().count()
    }

    /// The entity with the given GUID, if it exists.
    pub fn entity_of(&self, guid: EntityGuid) -> Option<Entity> {
        let world = self.ctx.world.lock();
        let mut index = GuidIndex::default();
        index.rebuild(&world);
        index.get(guid)
    }

    /// The GUID of an entity, if it has one.
    pub fn guid_of(&self, entity: Entity) -> Option<EntityGuid> {
        self.ctx
            .world
            .lock()
            .get_component::<EntityGuid>(entity)
            .copied()
    }

    /// GUIDs of every entity that has one.
    pub fn all_guids(&self) -> Vec<EntityGuid> {
        let world = self.ctx.world.lock();
        world
            .all_entities()
            .filter_map(|e| world.get_component::<EntityGuid>(e).copied())
            .collect()
    }

    /// The `Name` of an entity.
    pub fn name_of(&self, guid: EntityGuid) -> Option<String> {
        let entity = self.entity_of(guid)?;
        self.ctx
            .world
            .lock()
            .get_component::<Name>(entity)
            .map(|n| n.0.clone())
    }

    /// The first entity with the given name.
    pub fn find_by_name(&self, name: &str) -> Option<EntityGuid> {
        self.all_guids()
            .into_iter()
            .find(|&guid| self.name_of(guid).as_deref() == Some(name))
    }

    /// The parent of an entity, by GUID.
    pub fn parent_of(&self, guid: EntityGuid) -> Option<EntityGuid> {
        let entity = self.entity_of(guid)?;
        let parent = self
            .ctx
            .world
            .lock()
            .get_component::<Parent>(entity)
            .map(|p| p.0)?;
        self.guid_of(parent)
    }

    /// Makes the entity the primary selection.
    pub fn select(&self, guid: EntityGuid) {
        let entity = self.entity_of(guid);
        self.ctx.selection.lock().set_primary(entity);
    }

    /// Creates an entity through the `create_entity` command and gives it `name`.
    pub fn create_entity(&self, name: &str) -> Result<EntityGuid, String> {
        if !self.run("create_entity") {
            return Err("create_entity is not registered".into());
        }
        let entity = self
            .ctx
            .selection
            .lock()
            .primary
            .ok_or("nothing is selected after create_entity")?;
        let guid = self.guid_of(entity).ok_or("the new entity has no GUID")?;
        self.rename(guid, name)?;
        Ok(guid)
    }

    /// Duplicates the entity through the `duplicate` command. Returns the copy's GUID.
    pub fn duplicate(&self, guid: EntityGuid) -> Result<EntityGuid, String> {
        self.select(guid);
        if !self.run("duplicate") {
            return Err("duplicate is not registered".into());
        }
        let copy = self
            .ctx
            .selection
            .lock()
            .primary
            .ok_or("nothing is selected after duplicate")?;
        self.guid_of(copy).ok_or("the copy has no GUID".into())
    }

    /// Deletes the entity through the `delete` command.
    pub fn delete(&self, guid: EntityGuid) -> Result<(), String> {
        self.select(guid);
        if self.run("delete") {
            Ok(())
        } else {
            Err("delete is not registered".into())
        }
    }

    /// Renames an entity as one undoable step.
    pub fn rename(&self, guid: EntityGuid, name: &str) -> Result<(), String> {
        let entity = self.entity_of(guid).ok_or("no such entity")?;
        let before = self.component_value(entity, "Name");
        let after = Name(name.to_owned()).to_value();
        self.commit_edit(
            "Rename",
            UndoCommand::SetComponent {
                guid,
                type_name: "Name".into(),
                before,
                after: Some(after),
            },
        )
    }

    /// Moves an entity to `position` as one undoable step.
    pub fn move_to(&self, guid: EntityGuid, position: [f32; 3]) -> Result<(), String> {
        let entity = self.entity_of(guid).ok_or("no such entity")?;
        let before = self.component_value(entity, "Transform");
        let mut transform = self
            .ctx
            .world
            .lock()
            .get_component::<Transform>(entity)
            .copied()
            .ok_or("the entity has no transform")?;
        transform.translation = Vec3::from_array(position);
        let after = transform.to_value();
        self.commit_edit(
            "Move",
            UndoCommand::SetComponent {
                guid,
                type_name: "Transform".into(),
                before,
                after: Some(after),
            },
        )
    }

    /// Sets the parent of `child` (or removes it) as one undoable step, keeping the world pose.
    pub fn reparent(
        &self,
        child: EntityGuid,
        new_parent: Option<EntityGuid>,
    ) -> Result<(), String> {
        let command = {
            let world = self.ctx.world.lock();
            UndoCommand::reparent(&world, child, new_parent, true).map_err(|e| format!("{e:?}"))?
        };
        self.commit_edit("Reparent", command)
    }

    /// Adds a component with the given fields to an entity as one undoable step.
    pub fn add_component(
        &self,
        guid: EntityGuid,
        type_name: &str,
        fields: Vec<(&str, ReflectValue)>,
    ) -> Result<(), String> {
        let entity = self.entity_of(guid).ok_or("no such entity")?;
        let before = self.component_value(entity, type_name);
        let after = ReflectValue::Struct {
            type_name: type_name.to_owned(),
            fields: fields
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
        };
        self.commit_edit(
            "Add Component",
            UndoCommand::SetComponent {
                guid,
                type_name: type_name.to_owned(),
                before,
                after: Some(after),
            },
        )
    }

    /// Undoes the last step. `false` when there is nothing to undo.
    pub fn undo(&self) -> bool {
        let mut world = self.ctx.world.lock();
        let registry = self.ctx.registry.lock();
        let mut history = self.ctx.undo.lock();
        history.undo(&mut world, &registry)
    }

    /// Redoes the last undone step. `false` when there is nothing to redo.
    pub fn redo(&self) -> bool {
        let mut world = self.ctx.world.lock();
        let registry = self.ctx.registry.lock();
        let mut history = self.ctx.undo.lock();
        history.redo(&mut world, &registry)
    }

    /// Undoes until nothing is left to undo.
    pub fn undo_all(&self) {
        while self.undo() {}
    }

    /// Forgets the undo history without touching the world.
    pub fn clear_history(&self) {
        *self.ctx.undo.lock() = UndoStack::new(HISTORY_LIMIT);
    }

    /// Starts Play in the editor.
    pub fn play(&mut self) {
        let world = self.ctx.world.lock();
        let registry = self.ctx.registry.lock();
        self.pie.start(&world, &registry);
    }

    /// Stops Play and restores the world taken at start.
    pub fn stop(&mut self) {
        let mut world = self.ctx.world.lock();
        let registry = self.ctx.registry.lock();
        self.pie.stop(&mut world, &registry);
    }

    /// Creates an entity that exists only inside the editor (for example a helper) and returns
    /// its handle. It has no GUID until something assigns one.
    pub fn spawn_editor_only(&self) -> Entity {
        let mut world = self.ctx.world.lock();
        let entity = world.spawn();
        world.add_component(entity, EditorOnly);
        world.add_component(entity, Transform::default());
        entity
    }

    /// Number of entities marked [`EditorOnly`].
    pub fn editor_only_count(&self) -> usize {
        editor_only_entities(&self.ctx.world.lock()).len()
    }

    /// Creates a plain entity with a transform directly in the world, without history. Used
    /// for setup in benchmarks and for changes made during Play.
    pub fn spawn_raw(&self) -> Entity {
        let mut world = self.ctx.world.lock();
        let entity = world.spawn();
        world.add_component(entity, Transform::default());
        entity
    }

    /// Creates `count` entities with GUIDs, names and transforms; every entity after the first
    /// ten is a child of the entity ten places before it. Used only for benchmarks.
    pub fn spawn_bulk(&self, count: usize) {
        let mut world = self.ctx.world.lock();
        let mut created: Vec<Entity> = Vec::with_capacity(count);
        for index in 0..count {
            let entity = world.spawn();
            world.add_component(entity, EntityGuid::new_random());
            world.add_component(entity, Name(format!("entity {index}")));
            world.add_component(entity, Transform::default());
            if index >= 10 {
                set_parent(&mut world, entity, created[index - 10]);
            }
            created.push(entity);
        }
    }

    /// Advances the test clock. Nothing else happens until [`Self::tick`].
    pub fn advance_clock(&self, secs: u64) {
        self.clock.advance(secs);
    }

    /// One turn of the main loop: autosave (not during Play) and the asset service.
    pub fn tick(&mut self) {
        let playing = self.pie.is_running();
        if !playing
            && let (Some(level), Some(service)) = (self.level.as_ref(), self.autosave.as_mut())
            && service.is_session_active()
        {
            let source = HarnessSource {
                ctx: &self.ctx,
                level,
            };
            let transactions = self.ctx.undo.lock().len() as u64;
            service.tick(&source, transactions);
        }
        if let Some(assets) = self.assets.as_mut() {
            let _changes = assets.tick();
        }
    }

    /// Files of the autosave folder of the open project (`*.scene.ron`).
    pub fn autosave_files(&self) -> Vec<PathBuf> {
        list_files(
            &self.project_dir().join(SERVICE_DIR).join("autosave"),
            false,
        )
        .into_iter()
        .filter(|p| p.to_string_lossy().ends_with(".scene.ron"))
        .collect()
    }

    /// Files under `.redox/autosave/discarded/`, all times.
    pub fn discarded_files(&self) -> Vec<PathBuf> {
        list_files(
            &self
                .project_dir()
                .join(SERVICE_DIR)
                .join("autosave")
                .join("discarded"),
            true,
        )
    }

    /// Whether `.redox/session.lock` exists.
    pub fn session_lock_exists(&self) -> bool {
        self.project_dir()
            .join(SERVICE_DIR)
            .join("session.lock")
            .exists()
    }

    /// The recovery offer of the last project open, if any.
    pub fn recovery_offer(&self) -> Option<&RecoveryOffer> {
        self.recovery.as_ref()
    }

    /// Accepts the recovery offer: the autosaved state becomes the open level, marked dirty.
    pub fn accept_recovery(&mut self) -> Result<(), String> {
        let offer = self.recovery.take().ok_or("no recovery offer")?;
        let dir = self.project_dir();
        let now = self.clock.now_secs();
        let bytes = apply_recovery(&dir, &offer, RecoveryChoice::Recover, now)
            .map_err(|e| format!("{e:?}"))?
            .ok_or("the autosave has no bytes")?;
        let mut scratch = World::new();
        let level = {
            let registry = self.ctx.registry.lock();
            match offer.level_path.as_deref() {
                Some(path) => {
                    Level::open(path, &registry, &mut scratch)
                        .map_err(|e| format!("{e:?}"))?
                        .0
                }
                None => Level::new_scene(&offer.level, &registry, &scratch),
            }
        };
        let tmp = self.root().join("recovered.scene.ron");
        std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
        *self.ctx.world.lock() = World::new();
        *self.ctx.undo.lock() = UndoStack::new(HISTORY_LIMIT);
        self.ctx.selection.lock().clear();
        {
            let registry = self.ctx.registry.lock();
            let mut world = self.ctx.world.lock();
            SceneDocument::open(&tmp, &registry, &mut world).map_err(|e| format!("{e:?}"))?;
        }
        let _ = std::fs::remove_file(&tmp);
        self.level = Some(level);
        self.begin_autosave_session();
        Ok(())
    }

    /// Declines the recovery offer: the autosaves move to `discarded/`.
    pub fn discard_recovery(&mut self) -> Result<(), String> {
        let offer = self.recovery.take().ok_or("no recovery offer")?;
        let dir = self.project_dir();
        let now = self.clock.now_secs();
        apply_recovery(&dir, &offer, RecoveryChoice::Discard, now).map_err(|e| format!("{e:?}"))?;
        self.begin_autosave_session();
        Ok(())
    }

    /// Writes a file under the project root, creating folders.
    pub fn write_file(&self, relative: &str, bytes: &[u8]) {
        let path = self.project_dir().join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("the folder is created");
        }
        std::fs::write(path, bytes).expect("the file is written");
    }

    /// Changes the content of an asset by one appended byte, so the watcher sees a change.
    pub fn touch_asset(&self, relative: &str) {
        let path = self.project_dir().join(relative);
        let mut bytes = std::fs::read(&path).expect("the asset exists");
        bytes.push(0);
        std::fs::write(path, bytes).expect("the asset is written");
    }

    /// Bytes of a project file.
    pub fn read_file(&self, relative: &str) -> Vec<u8> {
        std::fs::read(self.project_dir().join(relative)).expect("the file exists")
    }

    /// Whether a project file exists.
    pub fn file_exists(&self, relative: &str) -> bool {
        self.project_dir().join(relative).exists()
    }

    /// Queues an asset for import.
    pub fn request_import(&mut self, relative: &str) {
        if let Some(assets) = self.assets.as_mut() {
            assets.import(relative);
        }
    }

    /// Queues a failed asset for import again.
    pub fn retry_import(&mut self, relative: &str) {
        if let Some(assets) = self.assets.as_mut() {
            assets.retry_import(relative);
        }
    }

    /// Turns the main loop until the import queue is idle. Panics if it never becomes idle.
    pub fn wait_imports_idle(&mut self) {
        for _ in 0..IMPORT_WAIT_ROUNDS {
            self.tick();
            if self
                .assets
                .as_ref()
                .is_some_and(|a| a.import_counts().importing == 0)
            {
                for _ in 0..3 {
                    self.tick();
                }
                return;
            }
            std::thread::yield_now();
        }
        panic!("the import queue did not become idle");
    }

    /// Import status of an asset, as its debug text (for example `Imported`, `Failed("...")`).
    pub fn import_status(&self, relative: &str) -> Option<String> {
        self.assets
            .as_ref()?
            .status(relative)
            .map(|status: &AssetStatus| format!("{status:?}"))
    }

    /// Moves or renames an asset through the asset service. `confirm` accepts rewriting the
    /// references in closed scenes.
    pub fn move_asset(&mut self, from: &str, to: &str, confirm: bool) -> Result<(), String> {
        let assets = self.assets.as_mut().ok_or("no project is open")?;
        let report = assets.move_asset(from, to).map_err(|e| format!("{e:?}"))?;
        if confirm {
            assets
                .update_path_hints(report.guid)
                .map_err(|e| format!("{e:?}"))?;
        }
        Ok(())
    }

    /// Deletes an asset. Without `force` a referenced asset is not removed.
    pub fn delete_asset(&mut self, relative: &str, force: bool) -> Result<Deletion, String> {
        let assets = self.assets.as_mut().ok_or("no project is open")?;
        match assets
            .delete_asset(relative, force)
            .map_err(|e| format!("{e:?}"))?
        {
            DeleteOutcome::Deleted { .. } => Ok(Deletion::Deleted),
            DeleteOutcome::Blocked { dependents } => Ok(Deletion::Blocked(
                dependents.into_iter().map(|d| d.scene).collect(),
            )),
        }
    }

    /// Scene paths (relative) that reference the asset at `relative`.
    pub fn dependent_scenes(&self, relative: &str) -> Vec<String> {
        let Some(assets) = self.assets.as_ref() else {
            return Vec::new();
        };
        let Some(guid) = assets.guid_of(relative) else {
            return Vec::new();
        };
        assets
            .dependents(guid)
            .into_iter()
            .map(|d| d.scene)
            .collect()
    }

    /// The asset reference of a component: `(guid text, path text)`.
    pub fn asset_ref(
        &self,
        guid: EntityGuid,
        type_name: &str,
    ) -> Option<(Option<String>, Option<String>)> {
        let entity = self.entity_of(guid)?;
        let value = self.component_value(entity, type_name)?;
        let text = |field: &str| {
            value
                .field(field)
                .and_then(ReflectValue::as_string)
                .map(str::to_owned)
        };
        Some((text("guid"), text("path")))
    }

    /// Writes autosave settings into `editor_settings.ron`.
    pub fn save_autosave_policy(&self, policy: AutosavePolicy) -> Result<(), String> {
        std::fs::create_dir_all(self.config_dir()).map_err(|e| e.to_string())?;
        EditorSettings { autosave: policy }
            .save(&self.config_dir().join("editor_settings.ron"))
            .map_err(|e| format!("{e:?}"))
    }

    /// The autosave policy read from `editor_settings.ron` (default when the file is missing).
    pub fn autosave_policy(&self) -> AutosavePolicy {
        EditorSettings::load(&self.config_dir().join("editor_settings.ron"))
            .map(|settings| settings.autosave)
            .unwrap_or_default()
    }

    /// Writes raw bytes into the config folder.
    pub fn write_config(&self, name: &str, bytes: &[u8]) {
        let dir = self.config_dir();
        std::fs::create_dir_all(&dir).expect("the config folder is created");
        std::fs::write(dir.join(name), bytes).expect("the config file is written");
    }

    /// Loads `editor_layout.ron` the way the app does at start.
    pub fn load_layout(&self) -> LayoutOutcome {
        let path = self.config_dir().join("editor_layout.ron");
        let broken = self.config_dir().join("editor_layout.ron.broken");
        let _dock = redox_editor_ui::EditorDock::load_or_default(&path, &EditorLog::new());
        LayoutOutcome {
            used_default: !path.exists(),
            broken_renamed: broken.exists(),
        }
    }

    fn component_value(&self, entity: Entity, type_name: &str) -> Option<ReflectValue> {
        let world = self.ctx.world.lock();
        let registry = self.ctx.registry.lock();
        registry.read_component(&world, entity, type_name)
    }

    /// Applies one command and records it as one undo step under `label`. On failure nothing
    /// stays applied and the error text is returned.
    fn commit_edit(&self, label: &str, command: UndoCommand) -> Result<(), String> {
        let mut world = self.ctx.world.lock();
        let registry = self.ctx.registry.lock();
        let mut history = self.ctx.undo.lock();
        history.begin(label);
        if let Err(error) = command.apply(&mut world, &registry) {
            history.cancel(&mut world, &registry);
            return Err(format!("{error:?}"));
        }
        history.push(command);
        history.commit();
        Ok(())
    }
}

/// The name of the enum variant of `value`, taken from its debug text.
fn variant_name<T: std::fmt::Debug>(value: &T) -> String {
    format!("{value:?}")
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

/// Autosave source of the harness: the open level and the edited world.
struct HarnessSource<'a> {
    ctx: &'a EditorContext,
    level: &'a Level,
}

impl AutosaveSource for HarnessSource<'_> {
    fn is_dirty(&self) -> bool {
        let world = self.ctx.world.lock();
        let registry = self.ctx.registry.lock();
        self.level.is_dirty(&registry, &world)
    }

    fn level_name(&self) -> String {
        self.level.title()
    }

    fn level_path(&self) -> Option<PathBuf> {
        self.level.path().map(Path::to_path_buf)
    }

    fn snapshot_bytes(&self) -> Result<Vec<u8>, String> {
        let registry = self.ctx.registry.lock();
        let mut world = self.ctx.world.lock();
        let tmp = std::env::temp_dir().join(format!(
            "redox_autosave_{}_{:p}.scene.ron",
            std::process::id(),
            self
        ));
        SceneDocument::new(self.level.title())
            .save_as(&tmp, &registry, &mut world)
            .map_err(|e| e.to_string())?;
        let bytes = std::fs::read(&tmp).map_err(|e| e.to_string());
        let _ = std::fs::remove_file(&tmp);
        bytes
    }
}
