//! Git status, commit and fast-forward pull for `*.scene.ron` and `project.ron`.

use std::path::{Path, PathBuf};

use redox_editor_core::project::MANIFEST_FILE;

use crate::format::SceneFile;
use crate::scene_diff::{SceneEntityChange, diff_scene_files};

/// Version-control summary for the open scene document and project manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SceneGitStatus {
    /// Primary scene file being tracked (usually the open `*.scene.ron`).
    pub path: PathBuf,
    /// Working tree differs from `HEAD` for the scene or `project.ron`.
    pub dirty: bool,
    /// Commits on the current branch that are not on the upstream tracking branch.
    pub ahead: usize,
    /// Commits on upstream that are not merged into the current branch.
    pub behind: usize,
    /// Short name of the current branch, or `HEAD` when detached.
    pub branch: String,
}

/// Errors from scene git operations (`editor-git` feature).
#[derive(Debug, thiserror::Error)]
pub enum SceneGitError {
    /// git2 is not compiled into this build.
    #[error("editor-git feature is disabled")]
    FeatureDisabled,
    /// No repository was discovered for the project root.
    #[error("no git repository for {}", root.display())]
    NoRepository {
        /// Project or world root that was searched.
        root: PathBuf,
    },
    /// The repository has no working directory (bare repo).
    #[error("bare git repository")]
    BareRepository,
    /// Path is outside the repository work tree.
    #[error("{} is outside the git work tree", path.display())]
    OutsideRepository {
        /// Absolute path that could not be relativized.
        path: PathBuf,
    },
    /// libgit2 returned an error.
    #[error("git error: {0}")]
    Git(#[from] git2::Error),
    /// Scene file could not be read or parsed.
    #[error("scene file at {}: {source}", path.display())]
    SceneIo {
        /// File that failed.
        path: PathBuf,
        /// Underlying error.
        source: SceneGitParseError,
    },
    /// `pull` refused because the merge would not fast-forward.
    #[error("pull refused: branch diverged from upstream (not fast-forward)")]
    NotFastForward,
    /// Current branch has no upstream configured.
    #[error("no upstream branch configured for pull")]
    NoUpstream,
    /// Commit message was empty.
    #[error("commit message must not be empty")]
    EmptyMessage,
}

/// Parse failures when loading a scene for diff.
#[derive(Debug, thiserror::Error)]
pub enum SceneGitParseError {
    /// I/O error.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// RON parse error.
    #[error("ron: {0}")]
    Ron(String),
}

/// Loads a scene file from disk.
pub fn load_scene_file(path: &Path) -> Result<SceneFile, SceneGitParseError> {
    let text = std::fs::read_to_string(path)?;
    ron::from_str(&text).map_err(|err| SceneGitParseError::Ron(err.to_string()))
}

/// Loads the committed (`HEAD`) revision of `path` when it exists.
#[cfg(feature = "editor-git")]
pub fn scene_file_at_head(
    repo: &git2::Repository,
    path: &Path,
) -> Result<Option<SceneFile>, SceneGitError> {
    let rel = path_in_workdir(repo, path)?;
    let spec = format!("HEAD:{}", rel.to_string_lossy());
    let obj = match repo.revparse_single(&spec) {
        Ok(obj) => obj,
        Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let blob = obj.peel_to_blob()?;
    let text = std::str::from_utf8(blob.content()).map_err(|_| SceneGitError::SceneIo {
        path: path.to_path_buf(),
        source: SceneGitParseError::Ron("HEAD blob was not UTF-8".into()),
    })?;
    let file: SceneFile = ron::from_str(text).map_err(|err| SceneGitError::SceneIo {
        path: path.to_path_buf(),
        source: SceneGitParseError::Ron(err.to_string()),
    })?;
    Ok(Some(file))
}

#[cfg(not(feature = "editor-git"))]
pub fn scene_file_at_head(
    _repo: &(),
    _path: &Path,
) -> Result<Option<SceneFile>, SceneGitError> {
    Err(SceneGitError::FeatureDisabled)
}

/// GUID-level diff between `HEAD` and the working-tree scene file.
#[cfg(feature = "editor-git")]
pub fn diff_scene_worktree_vs_head(
    repo: &git2::Repository,
    scene_path: &Path,
) -> Result<Vec<SceneEntityChange>, SceneGitError> {
    let head = scene_file_at_head(repo, scene_path)?.unwrap_or_else(|| SceneFile {
        format_version: 0,
        name: String::new(),
        entities: Vec::new(),
    });
    let worktree = load_scene_file(scene_path).map_err(|source| SceneGitError::SceneIo {
        path: scene_path.to_path_buf(),
        source,
    })?;
    Ok(diff_scene_files(&head, &worktree))
}

#[cfg(not(feature = "editor-git"))]
pub fn diff_scene_worktree_vs_head(
    _repo: &(),
    _scene_path: &Path,
) -> Result<Vec<SceneEntityChange>, SceneGitError> {
    Err(SceneGitError::FeatureDisabled)
}

/// Computes [`SceneGitStatus`] for the open scene and project manifest.
#[cfg(feature = "editor-git")]
pub fn scene_git_status(
    repo: &git2::Repository,
    scene_path: &Path,
    project_root: &Path,
) -> Result<SceneGitStatus, SceneGitError> {
    let manifest = project_root.join(MANIFEST_FILE);
    let scene_dirty = path_dirty(repo, scene_path)?;
    let manifest_dirty = path_dirty(repo, &manifest)?;
    let (ahead, behind, branch) = ahead_behind_and_branch(repo)?;
    Ok(SceneGitStatus {
        path: scene_path.to_path_buf(),
        dirty: scene_dirty || manifest_dirty,
        ahead,
        behind,
        branch,
    })
}

#[cfg(not(feature = "editor-git"))]
pub fn scene_git_status(
    _repo: &(),
    scene_path: &Path,
    _project_root: &Path,
) -> Result<SceneGitStatus, SceneGitError> {
    Err(SceneGitError::FeatureDisabled)
}

/// Stages `scene_path` and `project.ron`, then creates a commit (never force-pushes).
#[cfg(feature = "editor-git")]
pub fn commit_scene_files(
    repo: &git2::Repository,
    scene_path: &Path,
    project_root: &Path,
    message: &str,
) -> Result<(), SceneGitError> {
    if message.trim().is_empty() {
        return Err(SceneGitError::EmptyMessage);
    }
    let manifest = project_root.join(MANIFEST_FILE);
    let mut index = repo.index()?;
    for path in [scene_path, manifest.as_path()] {
        let rel = path_in_workdir(repo, path)?;
        index.add_path(&rel)?;
    }
    index.write()?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let sig = repo.signature().or_else(|_| {
        git2::Signature::now("RedOx Editor", "editor@redox.local")
    })?;
    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        message,
        &tree,
        parents.as_slice(),
    )?;
    Ok(())
}

#[cfg(not(feature = "editor-git"))]
pub fn commit_scene_files(
    _repo: &(),
    _scene_path: &Path,
    _project_root: &Path,
    _message: &str,
) -> Result<(), SceneGitError> {
    Err(SceneGitError::FeatureDisabled)
}

/// Fast-forward merge of the current branch's upstream; refuses diverged histories.
#[cfg(feature = "editor-git")]
pub fn pull_ff_only(repo: &git2::Repository) -> Result<(), SceneGitError> {
    let head = repo.head()?;
    let upstream = head.upstream().map_err(|_| SceneGitError::NoUpstream)?;
    let local_commit = head.peel_to_commit()?;
    let upstream_commit = upstream.get().peel_to_commit()?;
    let (ahead, behind) = repo.graph_ahead_behind(local_commit.id(), upstream_commit.id())?;
    if ahead > 0 {
        return Err(SceneGitError::NotFastForward);
    }
    if behind == 0 {
        return Ok(());
    }
    let annotated = repo.find_annotated_commit(upstream_commit.id())?;
    let mut merge_opts = git2::MergeOptions::new();
    merge_opts.set_flags(git2::MergeOptionsFlags::FAIL_IF_NOT_FAST_FORWARD);
    repo.merge(&[&annotated], Some(&mut merge_opts), None)?;
    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout.force();
    repo.checkout_head(Some(&mut checkout))?;
    Ok(())
}

#[cfg(not(feature = "editor-git"))]
pub fn pull_ff_only(_repo: &()) -> Result<(), SceneGitError> {
    Err(SceneGitError::FeatureDisabled)
}

#[cfg(feature = "editor-git")]
fn path_in_workdir(repo: &git2::Repository, path: &Path) -> Result<PathBuf, SceneGitError> {
    let workdir = repo.workdir().ok_or(SceneGitError::BareRepository)?;
    path.strip_prefix(workdir)
        .map(Path::to_path_buf)
        .map_err(|_| SceneGitError::OutsideRepository {
            path: path.to_path_buf(),
        })
}

#[cfg(feature = "editor-git")]
fn path_dirty(repo: &git2::Repository, path: &Path) -> Result<bool, SceneGitError> {
    let rel = path_in_workdir(repo, path)?;
    let pathspec = rel.to_string_lossy();
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true);
    opts.recurse_untracked_dirs(true);
    opts.pathspec(pathspec.as_ref());
    let statuses = repo.statuses(Some(&mut opts))?;
    let dirty = statuses
        .iter()
        .any(|entry| !matches!(entry.status(), git2::Status::CURRENT));
    Ok(dirty)
}

#[cfg(feature = "editor-git")]
fn ahead_behind_and_branch(repo: &git2::Repository) -> Result<(usize, usize, String), SceneGitError> {
    let head = repo.head()?;
    let branch = head
        .shorthand()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "HEAD".to_string());
    let local = head.peel_to_commit()?.id();
    let (ahead, behind) = match head.upstream() {
        Ok(upstream) => {
            let upstream_oid = upstream.get().peel_to_commit()?.id();
            repo.graph_ahead_behind(local, upstream_oid)?
        }
        Err(_) => (0, 0),
    };
    Ok((ahead, behind, branch))
}

/// Opens or discovers a repository rooted at or above `root`.
#[cfg(feature = "editor-git")]
pub fn open_repo(root: &Path) -> Option<git2::Repository> {
    git2::Repository::discover(root)
        .or_else(|_| git2::Repository::open(root))
        .ok()
}

#[cfg(not(feature = "editor-git"))]
pub fn open_repo(_root: &Path) -> Option<()> {
    None
}