//! `redox_cook audit` — offline asset audit JSON (EDT-C2 step 4).

use std::path::PathBuf;

use redox_asset::AssetRegistry;
use redox_editor_assets::{AuditConfig, export_audit_json, run_audit};

/// Parsed `audit` job.
#[derive(Debug, PartialEq, Eq)]
pub struct AuditJob {
    /// Assets folder.
    pub assets_dir: PathBuf,
    /// Write JSON to stdout when set.
    pub json: bool,
}

/// Runs the audit and optionally prints JSON.
pub fn run(job: &AuditJob) -> Result<String, String> {
    let (registry, _) = AssetRegistry::scan(job.assets_dir.clone()).map_err(|e| e.to_string())?;
    let report = run_audit(&registry, &AuditConfig::default());
    let text = export_audit_json(&report);
    if job.json {
        println!("{text}");
    }
    Ok(text)
}
