//! The File menu of the editor: items, the unsaved-changes prompt, path dialogs and the load
//! report. Requests come in through the queue from menu items, palette commands and hotkeys.
//! The dialog rules themselves live in `redox_editor_core::file_state`.

use std::path::{Path, PathBuf};

use redox_ecs::World;
use redox_editor_core::EditorContext;
use redox_editor_core::UndoStack;
use redox_editor_core::file_commands::{FileRequest, FileRequestQueue};
use redox_editor_core::file_state::{
    self, PendingAction, PromptAnswer, PromptOutcome, SaveTarget,
};
use redox_editor_core::project::{ProjectContext, RECENT_FILE, RecentProjects};
use redox_editor_core::project_template::ProjectTemplate;
use redox_editor_core::autosave::AutosaveSource;
use redox_editor_scene::{Level, LoadReport};

/// Window title when no level is open.
const EDITOR_TITLE: &str = "RedOx Studio — Editor";

/// What a path dialog is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PathPurpose {
    OpenLevel,
    SaveAs,
    OpenProject,
    NewProject,
}

impl PathPurpose {
    fn title(self) -> &'static str {
        match self {
            PathPurpose::OpenLevel => "Open Level",
            PathPurpose::SaveAs => "Save Level As",
            PathPurpose::OpenProject => "Open Project",
            PathPurpose::NewProject => "New Project",
        }
    }
}

/// A dialog that takes a path typed by the user.
struct PathDialog {
    purpose: PathPurpose,
    text: String,
    template: ProjectTemplate,
    error: Option<String>,
}

/// What the user did in a path dialog during one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DialogAction {
    Stay,
    Confirm,
    Cancel,
}

/// Level, project and dialog state behind the File menu.
pub struct FileMenu {
    queue: FileRequestQueue,
    level: Option<Level>,
    project: Option<ProjectContext>,
    recent: RecentProjects,
    prompt: Option<PendingAction>,
    dialog: Option<PathDialog>,
    report: Vec<String>,
    quit_requested: bool,
    shown_title: String,
}

impl FileMenu {
    /// Creates the menu state and reads the recent-projects list. A broken list is logged and
    /// replaced by an empty one.
    pub fn new(queue: FileRequestQueue, ctx: &EditorContext) -> Self {
        let recent = RecentProjects::load(RECENT_FILE).unwrap_or_else(|err| {
            ctx.log.warn(format!("recent projects ignored: {err}"));
            RecentProjects::default()
        });
        Self {
            queue,
            level: None,
            project: None,
            recent,
            prompt: None,
            dialog: None,
            report: Vec::new(),
            quit_requested: false,
            shown_title: String::new(),
        }
    }

    /// Queues `request` for the next [`Self::handle_requests`].
    pub fn push_request(&self, request: FileRequest) {
        self.queue.push(request);
    }

    /// The open scene level as an autosave source. `None` when no scene level is open (a world
    /// level writes its actors as they change and needs no autosave).
    pub fn autosave_source<'a>(&'a self, ctx: &'a EditorContext) -> Option<LevelAutosave<'a>> {
        let level = self.level.as_ref().filter(|level| !level.is_world())?;
        Some(LevelAutosave { ctx, level })
    }

    /// The project that is open, if any.
    pub fn project(&self) -> Option<&ProjectContext> {
        self.project.as_ref()
    }

    /// Draws the File menu items. Each item queues a request; the recent-project list opens a
    /// path dialog filled with the chosen folder.
    pub fn show_items(&mut self, ui: &mut egui::Ui) {
        let availability = file_state::availability(self.level.is_some());
        if ui.button("New Level").clicked() {
            self.queue.push(FileRequest::NewLevel);
        }
        if ui.button("Open Level...").clicked() {
            self.queue.push(FileRequest::OpenLevel);
        }
        if ui
            .add_enabled(availability.save, egui::Button::new("Save   Ctrl+S"))
            .clicked()
        {
            self.queue.push(FileRequest::Save);
        }
        if ui
            .add_enabled(availability.save_as, egui::Button::new("Save As..."))
            .clicked()
        {
            self.queue.push(FileRequest::SaveAs);
        }
        if ui
            .add_enabled(availability.close, egui::Button::new("Close Level"))
            .clicked()
        {
            self.queue.push(FileRequest::CloseLevel);
        }
        ui.separator();
        if ui.button("New Project...").clicked() {
            self.queue.push(FileRequest::NewProject);
        }
        if ui.button("Open Project...").clicked() {
            self.queue.push(FileRequest::OpenProject);
        }
        let mut picked: Option<String> = None;
        ui.menu_button("Recent Projects", |ui| {
            if self.recent.entries().is_empty() {
                ui.label("No recent projects");
            }
            for entry in self.recent.entries() {
                if ui.button(entry.as_str()).clicked() {
                    picked = Some(entry.clone());
                }
            }
        });
        if let Some(entry) = picked {
            self.open_path_dialog_with(PathPurpose::OpenProject, entry);
        }
    }

    /// Runs the requests queued since the last call.
    pub fn handle_requests(&mut self, ctx: &EditorContext) {
        for request in self.queue.take_all() {
            match request {
                FileRequest::NewLevel => self.gated(PendingAction::NewLevel, ctx),
                FileRequest::OpenLevel => self.gated(PendingAction::OpenLevel, ctx),
                FileRequest::CloseLevel => self.gated(PendingAction::CloseLevel, ctx),
                FileRequest::NewProject => self.gated(PendingAction::NewProject, ctx),
                FileRequest::OpenProject => self.gated(PendingAction::OpenProject, ctx),
                FileRequest::Save => {
                    self.save_level(ctx);
                }
                FileRequest::SaveAs => self.save_as_request(ctx),
            }
        }
    }

    /// Called when the window asks to close. Returns `true` when the window may close now;
    /// otherwise the unsaved-changes prompt is opened and a later confirmed Quit closes it.
    pub fn request_quit(&mut self, ctx: &EditorContext) -> bool {
        if self.prompt.is_some() {
            return false;
        }
        if file_state::needs_save_prompt(self.is_dirty(ctx)) {
            self.prompt = Some(PendingAction::Quit);
            false
        } else {
            true
        }
    }

    /// Whether a confirmed Quit is waiting. Clears the flag.
    pub fn take_quit_request(&mut self) -> bool {
        std::mem::take(&mut self.quit_requested)
    }

    /// The window title when it changed since the last call.
    pub fn take_title_change(&mut self, ctx: &EditorContext) -> Option<String> {
        let base = self
            .level
            .as_ref()
            .map(|level| level.title())
            .unwrap_or_else(|| EDITOR_TITLE.to_string());
        let title = file_state::window_title(&base, self.is_dirty(ctx));
        if title == self.shown_title {
            return None;
        }
        self.shown_title = title.clone();
        Some(title)
    }

    /// Draws the open dialogs: the unsaved-changes prompt, the path dialog and the load report.
    pub fn show_windows(&mut self, egui_ctx: &egui::Context, ctx: &EditorContext) {
        self.show_prompt(egui_ctx, ctx);
        self.show_path_dialog(egui_ctx, ctx);
        self.show_report(egui_ctx);
    }

    fn gated(&mut self, action: PendingAction, ctx: &EditorContext) {
        if self.prompt.is_some() {
            ctx.log.warn("answer the open prompt first");
            return;
        }
        if file_state::needs_save_prompt(self.is_dirty(ctx)) {
            self.prompt = Some(action);
        } else {
            self.perform(action, ctx);
        }
    }

    fn perform(&mut self, action: PendingAction, ctx: &EditorContext) {
        match action {
            PendingAction::NewLevel => self.new_level(ctx),
            PendingAction::OpenLevel => self.open_path_dialog(PathPurpose::OpenLevel),
            PendingAction::CloseLevel => self.close_level(ctx),
            PendingAction::OpenProject => self.open_path_dialog(PathPurpose::OpenProject),
            PendingAction::NewProject => self.open_path_dialog(PathPurpose::NewProject),
            PendingAction::Quit => self.quit_requested = true,
        }
    }

    fn answer_prompt(&mut self, answer: PromptAnswer, ctx: &EditorContext) {
        let Some(action) = self.prompt.take() else {
            return;
        };
        match file_state::resolve_prompt(answer) {
            PromptOutcome::Abort => {}
            PromptOutcome::Continue => self.perform(action, ctx),
            PromptOutcome::SaveThenContinue => {
                if self.save_level(ctx) {
                    self.perform(action, ctx);
                }
            }
        }
    }

    fn is_dirty(&self, ctx: &EditorContext) -> bool {
        let Some(level) = self.level.as_ref() else {
            return false;
        };
        let world = ctx.world.lock();
        let registry = ctx.registry.lock();
        level.is_dirty(&registry, &world)
    }

    fn save_as_request(&mut self, ctx: &EditorContext) {
        if self.level.is_none() {
            ctx.log.warn("no level is open");
            return;
        }
        self.open_path_dialog(PathPurpose::SaveAs);
    }

    /// Saves the level to its path. Returns `true` when the level was written. A level without
    /// a path opens the Save As dialog instead and returns `false`.
    fn save_level(&mut self, ctx: &EditorContext) -> bool {
        let Some(has_path) = self.level.as_ref().map(|level| level.path().is_some()) else {
            ctx.log.warn("no level is open");
            return false;
        };
        if file_state::save_target(has_path) == SaveTarget::AskPath {
            self.open_path_dialog(PathPurpose::SaveAs);
            return false;
        }
        let Some(level) = self.level.as_mut() else {
            return false;
        };
        let mut world = ctx.world.lock();
        let registry = ctx.registry.lock();
        match level.save(&registry, &mut world) {
            Ok(()) => {
                ctx.log.info(format!("saved {}", level.title()));
                true
            }
            Err(err) => {
                ctx.log.error(format!("save failed: {err}"));
                false
            }
        }
    }

    fn new_level(&mut self, ctx: &EditorContext) {
        let level = {
            let mut world = ctx.world.lock();
            let registry = ctx.registry.lock();
            *world = World::new();
            Level::new_scene("untitled", &registry, &world)
        };
        self.install_level(level, ctx);
        ctx.log.info("new level: untitled");
    }

    fn close_level(&mut self, ctx: &EditorContext) {
        if self.level.take().is_none() {
            ctx.log.warn("no level is open");
            return;
        }
        *ctx.world.lock() = World::new();
        ctx.selection.lock().clear();
        *ctx.undo.lock() = UndoStack::default();
        ctx.log.info("level closed");
    }

    fn install_level(&mut self, level: Level, ctx: &EditorContext) {
        self.level = Some(level);
        ctx.selection.lock().clear();
        *ctx.undo.lock() = UndoStack::default();
    }

    /// Opens a level into a fresh world; the current level stays if the open fails. Returns the
    /// load report lines.
    fn load_level(&mut self, path: &Path, ctx: &EditorContext) -> Result<Vec<String>, String> {
        let mut fresh = World::new();
        let (level, report) = {
            let registry = ctx.registry.lock();
            Level::open(path, &registry, &mut fresh).map_err(|err| err.to_string())?
        };
        *ctx.world.lock() = fresh;
        self.install_level(level, ctx);
        ctx.log.info(format!("opened {}", path.display()));
        Ok(report_lines(&report))
    }

    fn save_level_as(&mut self, path: &Path, ctx: &EditorContext) -> Result<(), String> {
        let Some(level) = self.level.as_mut() else {
            return Err("no level is open".to_string());
        };
        let mut world = ctx.world.lock();
        let registry = ctx.registry.lock();
        level
            .save_as(path, &registry, &mut world)
            .map_err(|err| err.to_string())?;
        ctx.log.info(format!("saved as {}", path.display()));
        Ok(())
    }

    fn open_project(&mut self, root: &Path, ctx: &EditorContext) -> Result<(), String> {
        let mut project = ProjectContext::open(root).map_err(|err| err.to_string())?;
        project.mark_opened().map_err(|err| err.to_string())?;
        self.adopt_project(project, ctx);
        Ok(())
    }

    fn new_project(
        &mut self,
        root: &Path,
        template: ProjectTemplate,
        ctx: &EditorContext,
    ) -> Result<(), String> {
        let name = root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".to_string());
        let project = ProjectContext::create(root, &name, template).map_err(|err| err.to_string())?;
        self.adopt_project(project, ctx);
        Ok(())
    }

    /// Makes `project` the current project, remembers it in the recent list and opens its
    /// start scene. Warnings of the project and of the scene form the report.
    fn adopt_project(&mut self, project: ProjectContext, ctx: &EditorContext) {
        let mut report: Vec<String> = project
            .warnings()
            .iter()
            .map(|warning| format!("{warning:?}"))
            .collect();
        let start_scene = project.start_scene_path().filter(|path| path.is_file());
        self.recent.push(project.root());
        self.save_recent(ctx);
        ctx.log.info(format!("project {} ready", project.manifest().name));
        self.project = Some(project);
        if let Some(scene) = start_scene {
            match self.load_level(&scene, ctx) {
                Ok(lines) => report.extend(lines),
                Err(message) => ctx.log.error(format!("start scene: {message}")),
            }
        }
        self.report = report;
    }

    fn save_recent(&self, ctx: &EditorContext) {
        if let Err(err) = self.recent.save(RECENT_FILE) {
            ctx.log.warn(format!("recent projects not saved: {err}"));
        }
    }

    fn open_path_dialog(&mut self, purpose: PathPurpose) {
        let text = match purpose {
            PathPurpose::SaveAs => self
                .level
                .as_ref()
                .and_then(|level| level.path())
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "untitled.scene.ron".to_string()),
            _ => String::new(),
        };
        self.open_path_dialog_with(purpose, text);
    }

    fn open_path_dialog_with(&mut self, purpose: PathPurpose, text: String) {
        self.dialog = Some(PathDialog {
            purpose,
            text,
            template: ProjectTemplate::ThreeDStarter,
            error: None,
        });
    }

    fn confirm_dialog(&mut self, dialog: &PathDialog, ctx: &EditorContext) -> Result<(), String> {
        let text = dialog.text.trim();
        if text.is_empty() {
            return Err("type a path first".to_string());
        }
        let path = PathBuf::from(text);
        match dialog.purpose {
            PathPurpose::OpenLevel => {
                self.report = self.load_level(&path, ctx)?;
                Ok(())
            }
            PathPurpose::SaveAs => self.save_level_as(&path, ctx),
            PathPurpose::OpenProject => self.open_project(&path, ctx),
            PathPurpose::NewProject => self.new_project(&path, dialog.template, ctx),
        }
    }

    fn show_prompt(&mut self, egui_ctx: &egui::Context, ctx: &EditorContext) {
        let Some(action) = self.prompt else {
            return;
        };
        let mut answer: Option<PromptAnswer> = None;
        egui::Window::new("Unsaved changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(egui_ctx, |ui| {
                ui.label(format!(
                    "Save changes to the level before {}?",
                    action_label(action)
                ));
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        answer = Some(PromptAnswer::Save);
                    }
                    if ui.button("Don't save").clicked() {
                        answer = Some(PromptAnswer::DontSave);
                    }
                    if ui.button("Cancel").clicked() {
                        answer = Some(PromptAnswer::Cancel);
                    }
                });
            });
        if let Some(answer) = answer {
            self.answer_prompt(answer, ctx);
        }
    }

    fn show_path_dialog(&mut self, egui_ctx: &egui::Context, ctx: &EditorContext) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let mut open = true;
        let mut action = DialogAction::Stay;
        egui::Window::new(dialog.purpose.title())
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(egui_ctx, |ui| {
                ui.label("Type a path: a folder for projects and worlds, a *.scene.ron file for scenes.");
                ui.add(egui::TextEdit::singleline(&mut dialog.text).desired_width(360.0));
                if dialog.purpose == PathPurpose::NewProject {
                    ui.horizontal(|ui| {
                        for template in ProjectTemplate::ALL {
                            let _ = ui.radio_value(&mut dialog.template, template, template.label());
                        }
                    });
                }
                if let Some(error) = &dialog.error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error.as_str());
                }
                ui.horizontal(|ui| {
                    if ui.button("Confirm").clicked() {
                        action = DialogAction::Confirm;
                    }
                    if ui.button("Cancel").clicked() {
                        action = DialogAction::Cancel;
                    }
                });
            });
        if !open {
            action = DialogAction::Cancel;
        }
        match action {
            DialogAction::Stay => self.dialog = Some(dialog),
            DialogAction::Cancel => {}
            DialogAction::Confirm => {
                if let Err(message) = self.confirm_dialog(&dialog, ctx) {
                    ctx.log.error(format!("dialog: {message}"));
                    dialog.error = Some(message);
                    self.dialog = Some(dialog);
                }
            }
        }
    }

    fn show_report(&mut self, egui_ctx: &egui::Context) {
        if self.report.is_empty() {
            return;
        }
        let mut close = false;
        egui::Window::new("Load report")
            .collapsible(false)
            .show(egui_ctx, |ui| {
                for line in &self.report {
                    ui.label(line.as_str());
                }
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        if close {
            self.report.clear();
        }
    }
}

/// One line per warning of a load report.
fn report_lines(report: &LoadReport) -> Vec<String> {
    report
        .warnings
        .iter()
        .map(|warning| format!("{warning:?}"))
        .collect()
}

/// The question in the prompt: what the user was about to do.
fn action_label(action: PendingAction) -> &'static str {
    match action {
        PendingAction::NewLevel => "creating a new level",
        PendingAction::OpenLevel => "opening another level",
        PendingAction::CloseLevel => "closing the level",
        PendingAction::OpenProject => "opening another project",
        PendingAction::NewProject => "creating a new project",
        PendingAction::Quit => "closing the editor",
    }
}

/// Autosave source of the open scene level: its path, dirty state and scene bytes.
pub struct LevelAutosave<'a> {
    ctx: &'a EditorContext,
    level: &'a Level,
}

impl AutosaveSource for LevelAutosave<'_> {
    fn is_dirty(&self) -> bool {
        let world = self.ctx.world.lock();
        let registry = self.ctx.registry.lock();
        self.level.is_dirty(&registry, &world)
    }

    fn level_name(&self) -> String {
        self.level.title()
    }

    fn level_path(&self) -> Option<std::path::PathBuf> {
        self.level.path().map(std::path::Path::to_path_buf)
    }

    fn snapshot_bytes(&self) -> Result<Vec<u8>, String> {
        let registry = self.ctx.registry.lock();
        let mut world = self.ctx.world.lock();
        self.level
            .snapshot_bytes(&registry, &mut world)
            .map_err(|err| err.to_string())
    }
}
