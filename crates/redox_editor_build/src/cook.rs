//! Cook: pack the plan, write `cook_report.json`, optional runtime scene snapshot metadata.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use redox_asset::{AssetRegistry, PakWriter};
use serde::{Deserialize, Serialize};

use crate::plan::{CookPlan, InclusionReason};

/// One line in the cook report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CookAssetEntry {
    /// Asset identity (32 hex digits).
    pub guid: String,
    /// Path relative to the assets root.
    pub path: String,
    /// Why the asset was included.
    pub reason: InclusionReason,
}

/// Result of [`cook_pack`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CookReport {
    /// Profile that was cooked.
    pub profile: String,
    /// Output `.pak` path.
    pub pak: String,
    /// Entries in the archive (matches plan).
    pub assets: Vec<CookAssetEntry>,
    /// Runtime entity count when a scene snapshot was taken (editor-only stripped).
    pub runtime_entities: Option<usize>,
}

/// Packs only the assets listed in `plan` into `out_pak` and writes `report_json` atomically.
pub fn cook_pack(
    registry: &AssetRegistry,
    plan: &CookPlan,
    profile_name: &str,
    out_pak: &Path,
    report_json: &Path,
    compress: bool,
    runtime_entities: Option<usize>,
) -> Result<CookReport, String> {
    let allowed: HashSet<String> = plan
        .assets
        .iter()
        .filter_map(|guid| registry.path_of(*guid).map(str::to_owned))
        .collect();

    let pak_tmp = temp_next_to(out_pak, "pak");
    let json_tmp = temp_next_to(report_json, "json");

    let mut writer = PakWriter::new();
    writer.set_compression(compress);
    pack_filtered(&mut writer, registry, &allowed)?;

    writer.write_to(&pak_tmp).map_err(|e| e.to_string())?;

    let mut assets = Vec::with_capacity(plan.assets.len());
    for guid in &plan.assets {
        let Some(path) = registry.path_of(*guid) else {
            continue;
        };
        let reason = plan
            .reasons
            .get(guid)
            .cloned()
            .unwrap_or(InclusionReason::ProfileForced);
        assets.push(CookAssetEntry {
            guid: guid.to_string(),
            path: path.to_owned(),
            reason,
        });
    }

    let report = CookReport {
        profile: profile_name.to_owned(),
        pak: out_pak.display().to_string(),
        assets,
        runtime_entities,
    };

    let json =
        serde_json::to_string_pretty(&report).map_err(|e| format!("serialize report: {e}"))?;
    fs::write(&json_tmp, json).map_err(|e| e.to_string())?;
    atomic_rename(&pak_tmp, out_pak)?;
    atomic_rename(&json_tmp, report_json)?;
    Ok(report)
}

fn pack_filtered(
    writer: &mut PakWriter,
    registry: &AssetRegistry,
    allowed: &HashSet<String>,
) -> Result<(), String> {
    let root = registry.root();
    let mut paths: Vec<&str> = allowed.iter().map(String::as_str).collect();
    paths.sort_unstable();
    for rel in paths {
        let abs = root.join(rel);
        let bytes = fs::read(&abs).map_err(|e| format!("{}: {e}", abs.display()))?;
        writer.add(rel, bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn temp_next_to(path: &Path, kind: &str) -> PathBuf {
    PathBuf::from(format!("{}.tmp-{kind}", path.display()))
}

fn atomic_rename(from: &Path, to: &Path) -> Result<(), String> {
    fs::rename(from, to).map_err(|e| format!("rename {} → {}: {e}", from.display(), to.display()))
}
