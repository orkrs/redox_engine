//! Scene git status, commit and pull tests (temp repositories).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use redox_editor_core::project::MANIFEST_FILE;
use redox_editor_scene::format::{SceneEntity, SceneFile};
use redox_editor_scene::{SceneGitError, WorldGitTracker, diff_scene_files, load_scene_file};
use redox_reflect::ReflectValue;
use redox_scene::EntityGuid;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "redox_scene_git_{tag}_{}_{nanos}",
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

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("run git");
    assert!(status.success(), "git {:?} in {}", args, dir.display());
}

fn init_repo(root: &Path) {
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.email", "test@redox.local"]);
    git(root, &["config", "user.name", "RedOx Test"]);
}

fn minimal_scene(path: &Path, name: &str) {
    let guid = EntityGuid::new_random();
    let file = SceneFile {
        format_version: 1,
        name: name.into(),
        entities: vec![SceneEntity {
            guid: Some(guid),
            components: vec![ReflectValue::Struct {
                type_name: "Name".into(),
                fields: vec![("name".into(), ReflectValue::String(name.into()))],
            }],
            unknown_components: Vec::new(),
        }],
    };
    let text = ron::ser::to_string_pretty(&file, ron::ser::PrettyConfig::new()).expect("ron");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create scenes dir");
    }
    fs::write(path, text).expect("write scene");
}

#[test]
fn diff_rename_entity_component_one_changed() {
    let guid = EntityGuid::new_random();
    let old = SceneFile {
        format_version: 1,
        name: "t".into(),
        entities: vec![SceneEntity {
            guid: Some(guid),
            components: vec![ReflectValue::Struct {
                type_name: "Name".into(),
                fields: vec![("name".into(), ReflectValue::String("old".into()))],
            }],
            unknown_components: Vec::new(),
        }],
    };
    let new = SceneFile {
        format_version: 1,
        name: "t".into(),
        entities: vec![SceneEntity {
            guid: Some(guid),
            components: vec![ReflectValue::Struct {
                type_name: "Name".into(),
                fields: vec![("name".into(), ReflectValue::String("new".into()))],
            }],
            unknown_components: Vec::new(),
        }],
    };
    let changes = diff_scene_files(&old, &new);
    assert_eq!(changes.len(), 1);
}

#[cfg(feature = "editor-git")]
mod git2_tests {
    use super::*;
    use redox_editor_scene::scene_git::{commit_scene_files, pull_ff_only, scene_git_status};

    #[test]
    fn modified_scene_file_is_dirty() {
        let dir = TempDir::new("dirty");
        let root = dir.path();
        init_repo(root);
        let scene = root.join("scenes/main.scene.ron");
        minimal_scene(&scene, "main");
        fs::write(root.join(MANIFEST_FILE), b"(name: \"p\")").expect("manifest");
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "init"]);

        let mut tracker = WorldGitTracker::new(root);
        tracker.set_tracked_scene(&scene, root);
        assert!(tracker.refresh_scene_git().is_ok());
        assert!(tracker.scene_status.as_ref().is_some_and(|s| !s.dirty));

        minimal_scene(&scene, "main_edited");
        tracker.refresh_scene_git().expect("refresh");
        assert!(tracker.scene_status.as_ref().is_some_and(|s| s.dirty));
    }

    #[test]
    fn commit_stages_scene_and_manifest() {
        let dir = TempDir::new("commit");
        let root = dir.path();
        init_repo(root);
        let scene = root.join("scenes/main.scene.ron");
        minimal_scene(&scene, "main");
        fs::write(root.join(MANIFEST_FILE), b"(name: \"p\")").expect("manifest");
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "init"]);

        minimal_scene(&scene, "main_v2");
        let repo = git2::Repository::open(root).expect("open repo");
        commit_scene_files(&repo, &scene, root, "save scene").expect("commit");
        let status = scene_git_status(&repo, &scene, root).expect("status");
        assert!(!status.dirty);
    }

    #[test]
    fn pull_rejects_diverged_history() {
        let dir = TempDir::new("ff");
        let root = dir.path();
        init_repo(root);
        let scene = root.join("scenes/main.scene.ron");
        minimal_scene(&scene, "v1");
        fs::write(root.join(MANIFEST_FILE), b"(name: \"p\")").expect("manifest");
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "base"]);

        git(root, &["branch", "upstream"]);
        git(root, &["checkout", "-b", "work"]);
        minimal_scene(&scene, "work");
        git(root, &["add", "scenes/main.scene.ron"]);
        git(root, &["commit", "-m", "work"]);

        git(root, &["checkout", "upstream"]);
        minimal_scene(&scene, "upstream");
        git(root, &["add", "scenes/main.scene.ron"]);
        git(root, &["commit", "-m", "upstream"]);

        git(root, &["checkout", "work"]);
        git(root, &["branch", "--set-upstream-to", "upstream", "work"]);

        let repo = git2::Repository::open(root).expect("open");
        let err = pull_ff_only(&repo).expect_err("should not ff");
        assert!(matches!(err, SceneGitError::NotFastForward));
    }

    #[test]
    fn worktree_diff_matches_disk() {
        let dir = TempDir::new("diff_head");
        let root = dir.path();
        init_repo(root);
        let scene = root.join("scenes/main.scene.ron");
        minimal_scene(&scene, "v1");
        fs::write(root.join(MANIFEST_FILE), b"(name: \"p\")").expect("manifest");
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "init"]);

        minimal_scene(&scene, "v2");
        let mut tracker = WorldGitTracker::new(root);
        tracker.set_tracked_scene(&scene, root);
        tracker.refresh_scene_git().expect("refresh");
        assert!(!tracker.scene_diff.is_empty());

        let disk = load_scene_file(&scene).expect("load");
        assert_eq!(disk.name, "v2");
    }
}
