use std::collections::HashMap;
use std::path::{Path, PathBuf};

use redox_ecs::World;
use redox_math::{Transform, Vec3};
use redox_scene::ofpa::{self, ActorRecord};
use redox_scene::{PartitionCell, StreamedActor};

#[derive(Clone, Debug)]
pub enum ActorGitStatus {
    Unmodified,
    Modified { old_transform: Transform },
    Added,
    Deleted { last_transform: Transform },
}

impl ActorGitStatus {
    pub fn badge(&self) -> &'static str {
        match self {
            Self::Unmodified => "🔵",
            Self::Modified { .. } => "🟡",
            Self::Added => "🟢",
            Self::Deleted { .. } => "🔴",
        }
    }

    pub fn is_dirty(&self) -> bool {
        !matches!(self, Self::Unmodified)
    }

    pub fn old_transform(&self) -> Option<Transform> {
        match self {
            Self::Modified { old_transform } => Some(*old_transform),
            Self::Deleted { last_transform } => Some(*last_transform),
            _ => None,
        }
    }
}

/// Tracks actor dirty state vs on-disk OFPA (and optional git2).
pub struct WorldGitTracker {
    #[cfg(feature = "editor-git")]
    repo: Option<git2::Repository>,
    pub status_cache: HashMap<String, ActorGitStatus>,
    pub world_root: PathBuf,
    /// Ghost transforms to draw (actor_id → old transform).
    pub ghosts: Vec<(String, Transform)>,
}

impl std::fmt::Debug for WorldGitTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("WorldGitTracker");
        d.field("status_cache", &self.status_cache)
            .field("world_root", &self.world_root)
            .field("ghosts", &self.ghosts);
        #[cfg(feature = "editor-git")]
        d.field("has_repo", &self.repo.is_some());
        d.finish()
    }
}

impl WorldGitTracker {
    pub fn new(world_root: impl AsRef<Path>) -> Self {
        Self {
            #[cfg(feature = "editor-git")]
            repo: git2::Repository::discover(world_root.as_ref())
                .or_else(|_| git2::Repository::open(world_root.as_ref()))
                .ok(),
            status_cache: HashMap::new(),
            world_root: world_root.as_ref().to_path_buf(),
            ghosts: Vec::new(),
        }
    }

    /// Rebind world root and rediscover git repo when `editor-git` is enabled.
    pub fn set_world_root(&mut self, world_root: impl AsRef<Path>) {
        self.world_root = world_root.as_ref().to_path_buf();
        #[cfg(feature = "editor-git")]
        {
            self.repo = git2::Repository::discover(&self.world_root)
                .or_else(|_| git2::Repository::open(&self.world_root))
                .ok();
        }
    }

    /// Compare live `StreamedActor` transforms to disk ActorRecords.
    pub fn refresh_from_world(&mut self, world: &World) {
        self.status_cache.clear();
        self.ghosts.clear();

        // Index disk actors
        let mut disk: HashMap<String, (PartitionCell, ActorRecord)> = HashMap::new();
        let cells_root = self.world_root.join(ofpa::CELLS_DIR);
        if let Ok(entries) = std::fs::read_dir(&cells_root) {
            for cell_dir in entries.flatten() {
                let Some(name) = cell_dir.file_name().to_str().map(|s| s.to_string()) else {
                    continue;
                };
                let Some(cell) = ofpa::parse_cell_dir(&name) else {
                    continue;
                };
                if let Ok(actors) = ofpa::load_cell_actors(&self.world_root, cell) {
                    for a in actors {
                        disk.insert(a.id.clone(), (cell, a));
                    }
                }
            }
        }

        let mut seen = std::collections::HashSet::new();
        for e in world.all_entities() {
            let Some(actor) = world.get_component::<StreamedActor>(e) else {
                continue;
            };
            let Some(xf) = world.get_component::<Transform>(e).copied() else {
                continue;
            };
            seen.insert(actor.id.clone());
            if let Some((_, disk_rec)) = disk.get(&actor.id) {
                let disk_pos = Vec3::from_array(disk_rec.position);
                let dirty = (xf.translation - disk_pos).length() > 0.05;
                if dirty {
                    let old = Transform {
                        translation: disk_pos,
                        rotation: redox_math::Quat::from_rotation_y(disk_rec.yaw),
                        scale: Vec3::ONE,
                    };
                    self.ghosts.push((actor.id.clone(), old));
                    self.status_cache.insert(
                        actor.id.clone(),
                        ActorGitStatus::Modified { old_transform: old },
                    );
                } else {
                    self.status_cache
                        .insert(actor.id.clone(), ActorGitStatus::Unmodified);
                }
            } else {
                self.status_cache
                    .insert(actor.id.clone(), ActorGitStatus::Added);
            }
        }

        // Brush-spawned without matching? already Added.
        // Disk-only actors not in world → Deleted (optional ghosts)
        for (id, (_, rec)) in &disk {
            if !seen.contains(id) {
                let old = Transform {
                    translation: Vec3::from_array(rec.position),
                    rotation: redox_math::Quat::from_rotation_y(rec.yaw),
                    scale: Vec3::ONE,
                };
                self.status_cache.insert(
                    id.clone(),
                    ActorGitStatus::Deleted {
                        last_transform: old,
                    },
                );
            }
        }

        #[cfg(feature = "editor-git")]
        {
            let _ = &self.repo;
        }
    }

    /// Alias for [`Self::refresh_from_world`] — integrator-friendly name.
    pub fn refresh(&mut self, world: &World) {
        self.refresh_from_world(world);
    }

    pub fn refresh_status(&mut self, world_root: &Path) {
        self.set_world_root(world_root);
        // Disk-only fallback when no world handed in
        self.status_cache.clear();
        self.ghosts.clear();
        if let Ok(entries) = std::fs::read_dir(world_root.join(ofpa::CELLS_DIR)) {
            for cell_dir in entries.flatten() {
                if let Ok(files) = std::fs::read_dir(cell_dir.path()) {
                    for file in files.flatten() {
                        if let Some(name) = file.file_name().to_str() {
                            let id = name.trim_end_matches(".ron").to_string();
                            self.status_cache
                                .entry(id)
                                .or_insert(ActorGitStatus::Unmodified);
                        }
                    }
                }
            }
        }
    }

    pub fn status_for(&self, actor_id: &str) -> &ActorGitStatus {
        self.status_cache
            .get(actor_id)
            .unwrap_or(&ActorGitStatus::Unmodified)
    }

    /// Sorted actor ids that are Modified / Added / Deleted.
    pub fn dirty_actor_ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self
            .status_cache
            .iter()
            .filter(|(_, s)| s.is_dirty())
            .map(|(id, _)| id.as_str())
            .collect();
        ids.sort_unstable();
        ids
    }

    pub fn revert_actor(&self, cell_x: i32, cell_y: i32, actor_id: &str) -> bool {
        let cell = PartitionCell {
            x: cell_x,
            y: cell_y,
        };
        let path = ofpa::actor_path(&self.world_root, cell, actor_id);
        #[cfg(feature = "editor-git")]
        {
            if let Some(repo) = &self.repo {
                let rel = path
                    .strip_prefix(repo.workdir().unwrap_or(Path::new("")))
                    .unwrap_or(&path);
                let mut checkout = git2::build::CheckoutBuilder::new();
                checkout.path(rel);
                if repo.checkout_head(Some(checkout.force())).is_ok() {
                    return true;
                }
            }
        }
        // Without git: reload from disk is a no-op if we haven't written dirty yet.
        // If file exists, "revert" means re-read — caller reloads cell.
        let _ = (cell_x, cell_y, actor_id, path);
        false
    }

    /// True when the `editor-git` feature compiled git2 into this build.
    pub fn is_available(&self) -> bool {
        cfg!(feature = "editor-git")
    }

    /// True when a git repository was discovered for the current world root.
    pub fn has_repo(&self) -> bool {
        #[cfg(feature = "editor-git")]
        {
            self.repo.is_some()
        }
        #[cfg(not(feature = "editor-git"))]
        {
            false
        }
    }

    /// Modified + Added (excludes Deleted). Prefer [`Self::dirty_count`] for overlay totals.
    pub fn modified_count(&self) -> usize {
        self.status_cache
            .values()
            .filter(|s| matches!(s, ActorGitStatus::Modified { .. } | ActorGitStatus::Added))
            .count()
    }

    /// All non-Unmodified actors (Modified + Added + Deleted).
    pub fn dirty_count(&self) -> usize {
        self.status_cache.values().filter(|s| s.is_dirty()).count()
    }

    /// One-line status for status bar / menus.
    pub fn summary(&self) -> String {
        let git = if self.is_available() {
            if self.has_repo() {
                "git2"
            } else {
                "git2 (no repo)"
            }
        } else {
            "OFPA"
        };
        format!(
            "{} dirty · {} · {}",
            self.dirty_count(),
            git,
            self.world_root.display()
        )
    }
}
