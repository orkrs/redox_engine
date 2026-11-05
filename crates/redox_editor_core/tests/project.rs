//! Tests of project files: creation, opening, templates, migration, warnings and recent projects.

use redox_editor_core::project::{
    ENGINE_VERSION, MANIFEST_FILE, PROJECT_FORMAT_VERSION, PRIVATE_DIR, ProjectContext,
    ProjectError, ProjectManifest, ProjectWarning, RECENT_LIMIT, RecentProjects, format_utc,
};
use redox_editor_core::project_template::ProjectTemplate;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A unique folder under the system temp directory, removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "redox_project_{tag}_{}_{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn manifest(name: &str, engine_version: &str, plugins: Vec<String>) -> ProjectManifest {
    ProjectManifest {
        format_version: PROJECT_FORMAT_VERSION,
        name: name.to_string(),
        engine_version: engine_version.to_string(),
        start_scene: String::new(),
        assets_root: "assets".to_string(),
        plugins,
        build_profiles: Vec::new(),
        created: "2026-01-01T00:00:00Z".to_string(),
        last_opened: "2026-01-01T00:00:00Z".to_string(),
    }
}

fn write_manifest(root: &Path, manifest: &ProjectManifest) {
    let text = ron::ser::to_string_pretty(manifest, ron::ser::PrettyConfig::default())
        .expect("serialize manifest");
    fs::write(root.join(MANIFEST_FILE), text).expect("write manifest");
}

#[test]
fn create_then_open_returns_equal_manifest() {
    let dir = TempDir::new("roundtrip");
    let created = ProjectContext::create(dir.path(), "Demo", ProjectTemplate::Empty)
        .expect("create project");
    let opened = ProjectContext::open(dir.path()).expect("open project");

    assert_eq!(created.manifest(), opened.manifest());
    assert_eq!(opened.manifest().name, "Demo");
    assert_eq!(opened.manifest().engine_version, ENGINE_VERSION);
    assert!(opened.warnings().is_empty());
    assert!(dir.path().join("scenes").is_dir());
    assert!(dir.path().join("assets").is_dir());
    assert!(dir.path().join(PRIVATE_DIR).join(".gitignore").is_file());
}

#[test]
fn creating_twice_in_the_same_folder_is_refused() {
    let dir = TempDir::new("twice");
    ProjectContext::create(dir.path(), "Demo", ProjectTemplate::Empty).expect("first create");
    let err = ProjectContext::create(dir.path(), "Demo", ProjectTemplate::Empty)
        .expect_err("second create must fail");
    assert!(matches!(err, ProjectError::AlreadyExists { .. }));
}

#[test]
fn three_d_starter_template_has_camera_light_and_cube() {
    let dir = TempDir::new("starter");
    let project = ProjectContext::create(dir.path(), "Starter", ProjectTemplate::ThreeDStarter)
        .expect("create project");
    let start = project.start_scene_path().expect("start scene");
    let text = fs::read_to_string(&start).expect("read start scene");

    assert!(text.contains("\"Camera\""));
    assert!(text.contains("\"DirectionalLight\""));
    assert!(text.contains("\"Cube\""));
    let scenes = project.scenes().expect("list scenes");
    assert_eq!(scenes, vec![start]);
}

#[test]
fn empty_template_has_no_cube() {
    let dir = TempDir::new("empty_template");
    let project =
        ProjectContext::create(dir.path(), "Empty", ProjectTemplate::Empty).expect("create");
    let text = fs::read_to_string(project.start_scene_path().expect("start scene"))
        .expect("read start scene");
    assert!(text.contains("\"Camera\""));
    assert!(!text.contains("\"Cube\""));
}

#[test]
fn manifest_from_a_newer_engine_is_refused() {
    let dir = TempDir::new("newer_engine");
    write_manifest(dir.path(), &manifest("Future", "99.0.0", Vec::new()));
    let err = ProjectContext::open(dir.path()).expect_err("newer engine must be refused");
    assert!(matches!(err, ProjectError::NewerEngine { .. }));
}

#[test]
fn newer_format_version_is_refused() {
    let dir = TempDir::new("newer_format");
    let mut newer = manifest("Future", ENGINE_VERSION, Vec::new());
    newer.format_version = PROJECT_FORMAT_VERSION + 1;
    write_manifest(dir.path(), &newer);
    let err = ProjectContext::open(dir.path()).expect_err("newer format must be refused");
    assert!(matches!(err, ProjectError::NewerFormat { .. }));
}

#[test]
fn missing_plugin_is_reported_and_project_still_opens() {
    let dir = TempDir::new("plugin");
    write_manifest(
        dir.path(),
        &manifest("Plugins", ENGINE_VERSION, vec!["x".to_string()]),
    );
    let project = ProjectContext::open(dir.path()).expect("project must still open");
    assert_eq!(
        project.warnings(),
        &[ProjectWarning::MissingPlugin {
            name: "x".to_string()
        }]
    );
}

#[test]
fn missing_start_scene_is_reported_and_project_opens_without_it() {
    let dir = TempDir::new("missing_start");
    let mut with_scene = manifest("Lost", ENGINE_VERSION, Vec::new());
    with_scene.start_scene = "scenes/gone.scene.ron".to_string();
    write_manifest(dir.path(), &with_scene);
    let project = ProjectContext::open(dir.path()).expect("project must still open");
    assert!(matches!(
        project.warnings(),
        [ProjectWarning::MissingStartScene { .. }]
    ));
}

#[test]
fn older_engine_migrates_and_keeps_the_first_backup() {
    let dir = TempDir::new("migrate");
    write_manifest(dir.path(), &manifest("Old", "0.0.1", Vec::new()));
    let original = fs::read_to_string(dir.path().join(MANIFEST_FILE)).expect("read original");

    let project = ProjectContext::open(dir.path()).expect("migrating open");
    assert_eq!(project.manifest().engine_version, ENGINE_VERSION);
    let backup = dir
        .path()
        .join(format!("{MANIFEST_FILE}.bak-v{PROJECT_FORMAT_VERSION}"));
    assert_eq!(fs::read_to_string(&backup).expect("backup exists"), original);

    // A second migration must not overwrite the first backup.
    write_manifest(dir.path(), &manifest("Older still", "0.0.2", Vec::new()));
    ProjectContext::open(dir.path()).expect("second migrating open");
    assert_eq!(fs::read_to_string(&backup).expect("backup kept"), original);
}

#[test]
fn recent_projects_keep_at_most_ten_without_duplicates() {
    let mut recent = RecentProjects::default();
    for index in 0..(RECENT_LIMIT + 2) {
        recent.push(Path::new(&format!("C:/projects/p{index}")));
    }
    recent.push(Path::new("C:/projects/p5"));

    assert_eq!(recent.entries().len(), RECENT_LIMIT);
    assert_eq!(recent.entries()[0], "C:/projects/p5");
    let unique: std::collections::HashSet<&String> = recent.entries().iter().collect();
    assert_eq!(unique.len(), recent.entries().len());
}

#[test]
fn recent_projects_round_trip_through_a_file() {
    let dir = TempDir::new("recent");
    let file = dir.path().join("editor_recent.ron");
    assert_eq!(
        RecentProjects::load(&file).expect("missing file is empty"),
        RecentProjects::default()
    );
    let mut recent = RecentProjects::default();
    recent.push(Path::new("C:/projects/a"));
    recent.save(&file).expect("save recent");
    assert_eq!(RecentProjects::load(&file).expect("load recent"), recent);
}

#[test]
fn format_utc_matches_known_instants() {
    assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(format_utc(1_700_000_000), "2023-11-14T22:13:20Z");
}
