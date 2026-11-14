//! `redox_cook import <project-root> [--path <file>]`

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use redox_asset::guid::AssetStatus;
use redox_editor_assets::{AssetService, ImportSettings};
use redox_editor_core::project::ProjectContext;
use serde::Serialize;

/// Parsed `import` command.
#[derive(Debug, PartialEq, Eq)]
pub struct ImportJob {
    /// Project root.
    pub root: PathBuf,
    /// Import only this path when set.
    pub path: Option<PathBuf>,
}

/// Outcome of one import CLI run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ImportCliReport {
    /// Paths imported successfully (relative to the assets root).
    pub imported: Vec<String>,
    /// Paths that failed.
    pub failed: Vec<ImportFailEntry>,
}

/// One failed import line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ImportFailEntry {
    pub path: String,
    pub message: String,
}

/// Parses `import` arguments.
pub fn parse_import(args: &[&str]) -> Result<ImportJob, String> {
    let Some(root) = args.first() else {
        return Err("import requires <project-root>".into());
    };
    let mut job = ImportJob {
        root: root.into(),
        path: None,
    };
    let mut iter = args.iter().skip(1);
    while let Some(flag) = iter.next() {
        match *flag {
            "--path" => {
                let rel = iter
                    .next()
                    .ok_or_else(|| "--path requires a file".to_string())?;
                job.path = Some(rel.into());
            }
            other => return Err(format!("unknown option: {other}")),
        }
    }
    Ok(job)
}

/// Runs the headless import queue to completion; exit ≠ 0 when any job failed.
pub fn run(job: &ImportJob) -> Result<ImportCliReport, String> {
    let project = ProjectContext::open(&job.root).map_err(|e| e.to_string())?;
    let assets = project.root().join(project.manifest().assets_root.as_str());
    let mut settings = ImportSettings::default();
    settings.auto_import = false;
    let mut service = AssetService::open_with(&assets, settings).map_err(|e| e.to_string())?;

    let mut touched = HashSet::new();
    if let Some(path) = &job.path {
        let rel = queue_file(&mut service, project.root(), path)?;
        touched.insert(rel);
    } else {
        let root = service.registry().root().to_path_buf();
        walk_importable(&root, &root, &mut service, &mut touched)?;
    }

    let mut report = ImportCliReport::default();
    wait_until_idle(&mut service);
    fill_report_from_registry(&service, &touched, &mut report);
    report.imported.sort();
    report.failed.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(report)
}

/// Exit code for an import report.
pub fn exit_code(report: &ImportCliReport) -> u8 {
    if report.failed.is_empty() { 0 } else { 1 }
}

fn queue_file(
    service: &mut AssetService,
    project_root: &Path,
    rel: &Path,
) -> Result<String, String> {
    let abs = if rel.is_absolute() {
        rel.to_path_buf()
    } else {
        project_root.join(rel)
    };
    let registry_root = service.registry().root();
    let registered = abs
        .strip_prefix(registry_root)
        .map_err(|_| format!("{} is outside the assets root", abs.display()))?;
    let rel = registered.to_string_lossy().replace('\\', "/");
    if !service.import(&rel) {
        return Err(format!("cannot queue import for {rel}"));
    }
    Ok(rel)
}

fn walk_importable(
    root: &Path,
    here: &Path,
    service: &mut AssetService,
    touched: &mut HashSet<String>,
) -> Result<(), String> {
    for entry in std::fs::read_dir(here).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            walk_importable(root, &path, service, touched)?;
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) == Some("meta") {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map_err(|_| "path outside assets root".to_string())?;
        let rel = rel.to_string_lossy().replace('\\', "/");
        touched.insert(rel.clone());
        let _ = service.import(&rel);
    }
    Ok(())
}

fn wait_until_idle(service: &mut AssetService) {
    for _ in 0..10_000 {
        let _ = service.tick();
        let counts = service.import_counts();
        if counts.importing == 0 {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
}

fn fill_report_from_registry(
    service: &AssetService,
    touched: &HashSet<String>,
    report: &mut ImportCliReport,
) {
    for path in touched {
        match service.registry().status(path.as_str()) {
            Some(AssetStatus::Imported) => report.imported.push(path.to_string()),
            Some(AssetStatus::Failed(message)) => report.failed.push(ImportFailEntry {
                path: path.to_string(),
                message: message.clone(),
            }),
            other => report.failed.push(ImportFailEntry {
                path: path.to_string(),
                message: other
                    .map(|status| format!("{status:?}"))
                    .unwrap_or_else(|| "asset is not registered".into()),
            }),
        }
    }
}

const MIN_PNG: &[u8] = &[
    0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H', b'D', b'R',
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xde, 0x00, 0x00, 0x00, 0x0c, b'I', b'D', b'A', b'T', 0x08, 0xd7, 0x63, 0xf8, 0xcf, 0xc0, 0x00,
    0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xdd, 0x8d, 0xb4, 0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N',
    b'D', 0xae, 0x42, 0x60, 0x82,
];

#[cfg(test)]
mod tests {
    use super::*;
    use redox_editor_core::project_template::ProjectTemplate;

    #[test]
    fn import_reports_success_and_failure() {
        let dir = std::env::temp_dir().join(format!("cook_import_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        ProjectContext::create(&dir, "Imp", ProjectTemplate::Empty).expect("create");
        let png = dir.join("assets/ok.png");
        std::fs::create_dir_all(png.parent().unwrap()).expect("assets");
        std::fs::write(&png, MIN_PNG).expect("png");
        let bad = dir.join("assets/bad.png");
        std::fs::write(&bad, b"not a texture").expect("bad");
        let report = run(&ImportJob {
            root: dir.clone(),
            path: None,
        })
        .expect("import run");
        assert!(!report.imported.is_empty());
        assert_eq!(report.failed.len(), 1);
        assert_ne!(exit_code(&report), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
