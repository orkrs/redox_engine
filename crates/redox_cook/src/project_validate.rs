//! `redox_cook validate <project-root> [--json <out>]`

use std::path::{Path, PathBuf};

use redox_editor_build::{validate_project, write_validation_json};

/// Parsed `validate` command.
#[derive(Debug, PartialEq, Eq)]
pub struct ValidateJob {
    /// Project root folder (contains `project.ron`).
    pub root: PathBuf,
    /// Optional JSON output path.
    pub json: Option<PathBuf>,
}

/// Parses arguments after `validate`.
pub fn parse_validate(args: &[&str]) -> Result<ValidateJob, String> {
    let Some(root) = args.first() else {
        return Err("validate requires <project-root>".into());
    };
    let mut job = ValidateJob {
        root: root.into(),
        json: None,
    };
    let mut iter = args.iter().skip(1);
    while let Some(flag) = iter.next() {
        match *flag {
            "--json" => {
                let out = iter
                    .next()
                    .ok_or_else(|| "--json requires a path".to_string())?;
                job.json = Some(out.into());
            }
            other => return Err(format!("unknown option: {other}")),
        }
    }
    Ok(job)
}

/// Runs validation; returns process exit code (0 = ok).
pub fn run(job: &ValidateJob) -> u8 {
    let Ok(report) = validate_project(&job.root) else {
        eprintln!("cannot open project at {}", job.root.display());
        return 1;
    };
    if let Some(path) = &job.json {
        if let Err(err) = write_validation_json(path, &report) {
            eprintln!("cannot write {}: {err}", path.display());
            return 1;
        }
    } else {
        for issue in &report.issues {
            let sev = match issue.severity {
                redox_editor_build::ValidationSeverity::Error => "error",
                redox_editor_build::ValidationSeverity::Warn => "warn",
            };
            eprintln!(
                "[{sev}] {} {:?} {:?} {}",
                issue.rule, issue.path, issue.guid, issue.message
            );
        }
    }
    report.exit_code()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn sample_project_has_exit_code_zero() {
        let root = std::env::temp_dir().join(format!("validate_ok_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let _ = redox_editor_core::project::ProjectContext::create(
            &root,
            "Ok",
            redox_editor_core::project_template::ProjectTemplate::ThreeDStarter,
        );
        assert_eq!(
            run(&ValidateJob {
                root: root.clone(),
                json: None
            }),
            0
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn broken_sample_fails_and_json_contains_missing_asset() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../redox_editor/tests/data/sample_project_broken");
        let json =
            std::env::temp_dir().join(format!("validate_broken_{}.json", std::process::id()));
        let code = run(&ValidateJob {
            root,
            json: Some(json.clone()),
        });
        assert_ne!(code, 0);
        let text = std::fs::read_to_string(&json).expect("json file");
        assert!(
            text.contains("MissingAsset"),
            "expected MissingAsset in {text}"
        );
        let _ = std::fs::remove_file(&json);
    }

    #[test]
    fn parse_accepts_json_flag() {
        let job = parse_validate(&["proj", "--json", "out.json"]).expect("parse");
        assert_eq!(job.json.as_deref(), Some(Path::new("out.json")));
    }
}
