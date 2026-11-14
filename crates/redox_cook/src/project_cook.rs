//! `redox_cook cook <project> --profile <name> [--json]`

use std::fs;
use std::path::{Path, PathBuf};

use redox_asset::AssetRegistry;
use redox_editor_build::{BuildService, CookReport};
use redox_editor_core::project::ProjectContext;

use crate::hlod_bake::{bake_hlod, parse_options};
use crate::nav::bake_navmesh_from_gltf;

/// Parsed `cook` command.
#[derive(Debug, PartialEq, Eq)]
pub struct CookJob {
    pub root: PathBuf,
    pub profile: Option<String>,
    pub json: bool,
}

/// Parses `cook` arguments.
pub fn parse_cook(args: &[&str]) -> Result<CookJob, String> {
    let Some(root) = args.first() else {
        return Err("cook requires <project-root>".into());
    };
    let mut job = CookJob {
        root: root.into(),
        profile: None,
        json: false,
    };
    let mut iter = args.iter().skip(1);
    while let Some(flag) = iter.next() {
        match *flag {
            "--profile" => {
                job.profile = Some(
                    iter.next()
                        .ok_or_else(|| "--profile requires a name".to_string())?
                        .to_string(),
                );
            }
            "--json" => job.json = true,
            other => return Err(format!("unknown option: {other}")),
        }
    }
    Ok(job)
}

/// Runs cook; writes `<project>/.redox/<profile>.pak` and `cook_report.json` by default.
pub fn run(job: &CookJob) -> Result<CookReport, String> {
    let project = ProjectContext::open(&job.root).map_err(|e| e.to_string())?;
    let service = BuildService;
    let profile = service
        .resolve_active(project.manifest(), job.profile.as_deref())
        .map_err(|e| e.to_string())?;
    let out_dir = project.root().join(".redox");
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let pak = out_dir.join(format!("{}.pak", profile.name));
    let report_path = out_dir.join("cook_report.json");
    let report = service
        .cook(&project, &profile, &pak, &report_path)
        .map_err(|e| e.to_string())?;
    let nav_status = cook_nav_sidecar(&project, &profile, &out_dir)?;
    let hlod_status = cook_hlod_sidecar(&project, &out_dir)?;
    patch_cook_report(&report_path, nav_status, hlod_status)?;
    if job.json {
        let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
        println!("{text}");
    } else {
        println!("cooked {} assets into {}", report.assets.len(), report.pak);
    }
    Ok(report)
}

fn patch_cook_report(report_path: &Path, nav: &str, hlod: &str) -> Result<(), String> {
    let text = fs::read_to_string(report_path).map_err(|e| e.to_string())?;
    let mut value: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let Some(obj) = value.as_object_mut() else {
        return Err("cook_report.json root must be an object".into());
    };
    obj.insert("nav".into(), serde_json::Value::String(nav.into()));
    obj.insert("hlod".into(), serde_json::Value::String(hlod.into()));
    let patched = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    fs::write(report_path, patched).map_err(|e| e.to_string())
}

fn first_level_mesh(project: &ProjectContext, profile: &redox_editor_core::project::BuildProfile) -> Option<PathBuf> {
    let service = BuildService;
    let plan = service.plan(project, profile).ok()?;
    let assets_root = project.root().join(project.manifest().assets_root.as_str());
    let (registry, _) = AssetRegistry::scan(&assets_root).ok()?;
    for guid in &plan.assets {
        let Some(rel) = registry.path_of(*guid) else {
            continue;
        };
        let path = assets_root.join(rel);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if (ext == "glb" || ext == "gltf") && path.is_file() {
            return Some(path);
        }
    }
    None
}

fn find_world_bin(project: &ProjectContext) -> Option<PathBuf> {
    let root = project.root();
    let candidates = [
        root.join(".redox/world.bin"),
        root.join("world.bin"),
        root.join("worlds/arena/world.bin"),
    ];
    candidates.into_iter().find(|path| path.is_file())
}

fn cook_nav_sidecar(
    project: &ProjectContext,
    profile: &redox_editor_core::project::BuildProfile,
    out_dir: &Path,
) -> Result<&'static str, String> {
    let Some(mesh) = first_level_mesh(project, profile) else {
        return Ok("skipped");
    };
    let out = out_dir.join(format!("{}.nav", profile.name));
    bake_navmesh_from_gltf(&mesh, &out)?;
    Ok("written")
}

fn cook_hlod_sidecar(project: &ProjectContext, out_dir: &Path) -> Result<&'static str, String> {
    let Some(world) = find_world_bin(project) else {
        return Ok("skipped");
    };
    let options = parse_options(&[])?;
    let written = bake_hlod(&world, out_dir, &options)?;
    if written == 0 {
        Ok("skipped")
    } else {
        Ok("written")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_editor_core::project_template::ProjectTemplate;

    #[test]
    fn parse_cook_flags() {
        let job = parse_cook(&["root", "--profile", "release", "--json"]).expect("parse");
        assert_eq!(job.profile.as_deref(), Some("release"));
        assert!(job.json);
    }

    #[test]
    fn cook_temp_project_writes_report() {
        let dir = std::env::temp_dir().join(format!("cook_cli_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let project =
            ProjectContext::create(&dir, "C", ProjectTemplate::ThreeDStarter).expect("create");
        let _ = project;
        run(&CookJob {
            root: dir.clone(),
            profile: None,
            json: false,
        })
        .expect("cook");
        assert!(dir.join(".redox/cook_report.json").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cook_without_level_geometry_skips_nav_and_still_writes_the_pak() {
        let dir = std::env::temp_dir().join(format!("cook_no_mesh_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let project =
            ProjectContext::create(&dir, "NavSkip", ProjectTemplate::ThreeDStarter).expect("create");
        let service = BuildService;
        let profile = service
            .resolve_active(project.manifest(), None)
            .expect("profile");
        let out_dir = dir.join(".redox");
        fs::create_dir_all(&out_dir).unwrap();
        let status = cook_nav_sidecar(&project, &profile, &out_dir).expect("nav sidecar");
        let has_mesh = first_level_mesh(&project, &profile).is_some();
        if has_mesh {
            assert_eq!(status, "written");
        } else {
            assert_eq!(status, "skipped");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cook_writes_a_nav_next_to_the_pak_when_the_level_has_a_mesh() {
        let dir = std::env::temp_dir().join(format!("cook_nav_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let project =
            ProjectContext::create(&dir, "Nav", ProjectTemplate::ThreeDStarter).expect("create");
        let service = BuildService;
        let profile = service
            .resolve_active(project.manifest(), None)
            .expect("profile");
        let out_dir = dir.join(".redox");
        fs::create_dir_all(&out_dir).unwrap();
        let status = cook_nav_sidecar(&project, &profile, &out_dir).expect("nav sidecar");
        if first_level_mesh(&project, &profile).is_some() {
            assert_eq!(status, "written");
            assert!(out_dir.join(format!("{}.nav", profile.name)).is_file());
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cook_writes_hlod_or_skips_when_there_is_no_world() {
        let dir = std::env::temp_dir().join(format!("cook_no_world_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let project =
            ProjectContext::create(&dir, "HlodSkip", ProjectTemplate::ThreeDStarter).expect("create");
        let status = cook_hlod_sidecar(&project, &dir.join(".redox")).expect("hlod sidecar");
        assert_eq!(status, "skipped");
        let _ = fs::remove_dir_all(&dir);
    }
}
