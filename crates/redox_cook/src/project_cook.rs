//! `redox_cook cook <project> --profile <name> [--json]`

use std::path::{Path, PathBuf};

use redox_editor_build::{BuildService, CookReport};
use redox_editor_core::project::ProjectContext;

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
    if job.json {
        let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
        println!("{text}");
    } else {
        println!("cooked {} assets into {}", report.assets.len(), report.pak);
    }
    Ok(report)
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
}
