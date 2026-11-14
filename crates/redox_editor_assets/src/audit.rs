//! Asset audit: reference viewer, orphans, duplicates, size map (EDT-C2).

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;

use redox_asset::{AssetGuid, AssetRegistry, content_hash};

use crate::browser_model::AssetKind;
use crate::import::probe_texture_header;
use crate::reference_index::{ReferenceIndex, is_scene_path};

/// Configuration for orphan detection.
#[derive(Clone, Debug, Default)]
pub struct AuditConfig {
    /// Assets kept even when no scene references them.
    pub force_keep: BTreeSet<AssetGuid>,
    /// Scene paths that define the used set (defaults to every indexed scene).
    pub scene_paths: Vec<String>,
}

/// Incoming reference from a scene file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncomingReference {
    /// Scene path relative to the assets root.
    pub scene: String,
    /// Entity guid text when known.
    pub entity: Option<String>,
    /// Component type name.
    pub component: String,
    /// Resolved target guid.
    pub guid: AssetGuid,
}

/// Outgoing reference from a scene asset to another asset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutgoingReference {
    /// Component holding the reference.
    pub component: String,
    /// Entity guid text when known.
    pub entity: Option<String>,
    /// Referenced guid when present in the file.
    pub guid: Option<String>,
    /// Path hint in the scene file.
    pub path: Option<String>,
}

/// Reference graph for one selected asset (EDT-C2 step 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferenceView {
    /// Selected asset.
    pub guid: AssetGuid,
    /// Current path when registered.
    pub path: Option<String>,
    /// Scene → asset uses.
    pub incoming: Vec<IncomingReference>,
    /// Asset → asset uses (scene files only in this slice).
    pub outgoing: Vec<OutgoingReference>,
}

impl Default for ReferenceView {
    fn default() -> Self {
        Self {
            guid: AssetGuid(0),
            path: None,
            incoming: Vec::new(),
            outgoing: Vec::new(),
        }
    }
}

/// UI-free reference viewer model.
#[derive(Clone, Debug, Default)]
pub struct ReferenceViewerModel {
    selected: Option<AssetGuid>,
    view: ReferenceView,
}

impl ReferenceViewerModel {
    /// Clears the selection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Selects a guid and rebuilds the view.
    pub fn select(&mut self, guid: AssetGuid, registry: &AssetRegistry, index: &ReferenceIndex) {
        self.selected = Some(guid);
        self.view = build_reference_view(guid, registry, index);
    }

    /// Currently selected guid.
    pub fn selected(&self) -> Option<AssetGuid> {
        self.selected
    }

    /// Latest reference view.
    pub fn view(&self) -> &ReferenceView {
        &self.view
    }
}

/// Builds incoming/outgoing reference lists for `guid`.
pub fn build_reference_view(
    guid: AssetGuid,
    registry: &AssetRegistry,
    index: &ReferenceIndex,
) -> ReferenceView {
    let path = registry.path_of(guid).map(str::to_owned);
    let incoming: Vec<IncomingReference> = registry
        .dependents(guid)
        .into_iter()
        .map(|dep| IncomingReference {
            scene: dep.scene,
            entity: dep.entity,
            component: dep.component,
            guid: dep.guid,
        })
        .collect();
    let outgoing = path
        .as_deref()
        .filter(|p| is_scene_path(p))
        .map(|scene| {
            index
                .refs_in_scene(scene)
                .iter()
                .map(|sref| OutgoingReference {
                    component: sref.component.clone(),
                    entity: sref.entity.clone(),
                    guid: sref.guid.clone(),
                    path: sref.path.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    ReferenceView {
        guid,
        path,
        incoming,
        outgoing,
    }
}

/// Orphan assets: not referenced from project scenes and not force-kept.
pub fn find_orphans(registry: &AssetRegistry, config: &AuditConfig) -> Vec<String> {
    let used = referenced_guids(registry, config);
    registry
        .iter()
        .filter(|(path, _guid)| !is_scene_path(path))
        .filter(|(_, guid)| !used.contains(guid) && !config.force_keep.contains(guid))
        .map(|(path, _)| path.to_owned())
        .collect()
}

fn referenced_guids(registry: &AssetRegistry, config: &AuditConfig) -> BTreeSet<AssetGuid> {
    let scenes: Vec<&str> = if config.scene_paths.is_empty() {
        registry
            .iter()
            .filter(|(p, _)| is_scene_path(p))
            .map(|(p, _)| p.as_ref())
            .collect()
    } else {
        config.scene_paths.iter().map(String::as_str).collect()
    };
    let mut used = BTreeSet::new();
    for scene in scenes {
        for dep in registry.scene_refs(scene) {
            used.insert(dep.guid);
        }
    }
    used
}

/// Duplicate group keyed by `content_hash`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuplicateGroup {
    /// Shared hash.
    pub hash: u64,
    /// Members with distinct guids.
    pub members: Vec<(AssetGuid, String)>,
}

/// Finds non-unique content by file bytes (EDT-C2 step 2).
pub fn find_duplicates(registry: &AssetRegistry) -> Vec<DuplicateGroup> {
    let mut map: HashMap<u64, Vec<(AssetGuid, String)>> = HashMap::new();
    for (path, guid) in registry.iter() {
        if is_scene_path(path) {
            continue;
        }
        let Ok(bytes) = fs::read(registry.root().join(path)) else {
            continue;
        };
        let hash = content_hash(&bytes);
        map.entry(hash).or_default().push((guid, path.to_owned()));
    }
    map.into_iter()
        .filter(|(_, members)| members.len() > 1)
        .map(|(hash, members)| DuplicateGroup { hash, members })
        .collect()
}

/// One node in the folder size tree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SizeMapNode {
    /// Folder path relative to the assets root (`""` for root).
    pub path: String,
    /// Sum of file bytes in this folder subtree.
    pub disk_bytes: u64,
    /// Rough GPU memory estimate when known.
    pub vram_bytes: Option<u64>,
    /// Child folders.
    pub children: Vec<SizeMapNode>,
}

/// Builds a folder tree with disk sizes and VRAM estimates.
pub fn build_size_map(registry: &AssetRegistry) -> SizeMapNode {
    let mut root = SizeMapNode {
        path: String::new(),
        disk_bytes: 0,
        vram_bytes: Some(0),
        children: Vec::new(),
    };
    for (path, _) in registry.iter() {
        if is_scene_path(path) {
            continue;
        }
        let Ok(meta) = fs::metadata(registry.root().join(path)) else {
            continue;
        };
        let bytes = meta.len();
        let vram = estimate_vram(registry, path);
        insert_size(&mut root, path, bytes, vram);
    }
    rollup_vram(&mut root);
    root
}

fn insert_size(node: &mut SizeMapNode, rel_path: &str, bytes: u64, vram: Option<u64>) {
    node.disk_bytes = node.disk_bytes.saturating_add(bytes);
    node.vram_bytes = add_opt(node.vram_bytes, vram);
    let Some((folder, rest)) = rel_path.split_once('/') else {
        return;
    };
    let idx = node
        .children
        .iter()
        .position(|child| child.path == folder)
        .unwrap_or_else(|| {
            node.children.push(SizeMapNode {
                path: folder.to_owned(),
                disk_bytes: 0,
                vram_bytes: Some(0),
                children: Vec::new(),
            });
            node.children.len() - 1
        });
    insert_size(&mut node.children[idx], rest, bytes, vram);
}

fn rollup_vram(node: &mut SizeMapNode) {
    for child in &mut node.children {
        rollup_vram(child);
    }
    if node.children.is_empty() {
        return;
    }
    let sum = node
        .children
        .iter()
        .filter_map(|c| c.vram_bytes)
        .fold(0_u64, u64::saturating_add);
    node.vram_bytes = Some(sum);
}

fn add_opt(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(a), Some(b)) => Some(a.saturating_add(b)),
        (None, _) | (_, None) => None,
    }
}

/// Rough bytes-on-GPU table for known texture headers; otherwise `None` (= unknown).
pub fn estimate_vram(registry: &AssetRegistry, path: &str) -> Option<u64> {
    let kind = AssetKind::from_path(Path::new(path));
    if kind != AssetKind::Texture {
        return None;
    }
    let bytes = fs::read(registry.root().join(path)).ok()?;
    let header = probe_texture_header(&bytes).ok()?;
    let texels = u64::from(header.width).saturating_mul(u64::from(header.height));
    let bpp = if header.format.contains("RGBA") {
        8
    } else if header.format.contains("RGB") {
        6
    } else if header.format.contains("KTX2") {
        4
    } else {
        return None;
    };
    let base = texels.saturating_mul(bpp);
    Some(base.saturating_mul(4) / 3)
}

/// Serialises the size map as compact JSON (no extra crate).
pub fn export_size_map_json(root: &SizeMapNode) -> String {
    fn write_node(node: &SizeMapNode, out: &mut String) {
        out.push('{');
        out.push_str("\"path\":\"");
        out.push_str(&escape_json(&node.path));
        out.push_str("\",\"disk_bytes\":");
        out.push_str(&node.disk_bytes.to_string());
        out.push_str(",\"vram_bytes\":");
        match node.vram_bytes {
            Some(v) => out.push_str(&v.to_string()),
            None => out.push_str("null"),
        }
        out.push_str(",\"children\":[");
        for (index, child) in node.children.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            write_node(child, out);
        }
        out.push_str("]}");
    }
    let mut out = String::new();
    write_node(root, &mut out);
    out
}

/// Serialises the size map as CSV rows (pre-order).
pub fn export_size_map_csv(root: &SizeMapNode) -> String {
    let mut lines = vec!["path,disk_bytes,vram_bytes".to_owned()];
    fn walk(node: &SizeMapNode, lines: &mut Vec<String>) {
        let vram = node
            .vram_bytes
            .map(|v| v.to_string())
            .unwrap_or_else(|| "unknown".to_owned());
        lines.push(format!(
            "{},{},{}",
            csv_escape(&node.path),
            node.disk_bytes,
            vram
        ));
        for child in &node.children {
            let child_path = if node.path.is_empty() {
                child.path.clone()
            } else {
                format!("{}/{}", node.path, child.path)
            };
            let mut normalized = child.clone();
            normalized.path = child_path;
            walk(&normalized, lines);
        }
    }
    walk(root, &mut lines);
    lines.join("\n")
}

/// One CSV row from [`export_size_map_csv`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SizeMapCsvRow {
    /// Folder path.
    pub path: String,
    /// Disk usage.
    pub disk_bytes: u64,
    /// VRAM estimate when known.
    pub vram_bytes: Option<u64>,
}

/// Parses CSV from [`export_size_map_csv`] (round-trip helper for tests).
pub fn parse_size_map_csv(text: &str) -> Result<Vec<SizeMapCsvRow>, String> {
    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if index == 0 && line.starts_with("path,") {
            continue;
        }
        let cols = split_csv_line(line);
        if cols.len() != 3 {
            return Err(format!("bad csv line: {line}"));
        }
        let disk: u64 = cols[1].parse().map_err(|_| "disk parse")?;
        let vram = if cols[2] == "unknown" {
            None
        } else {
            Some(cols[2].parse().map_err(|_| "vram parse")?)
        };
        rows.push(SizeMapCsvRow {
            path: cols[0].clone(),
            disk_bytes: disk,
            vram_bytes: vram,
        });
    }
    Ok(rows)
}

fn split_csv_line(line: &str) -> Vec<String> {
    let mut cols = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in line.chars() {
        match (ch, in_quotes) {
            ('"', true) => {
                in_quotes = false;
            }
            ('"', false) => {
                in_quotes = true;
            }
            (',', false) => {
                cols.push(current.clone());
                current.clear();
            }
            (_, _) => current.push(ch),
        }
    }
    cols.push(current);
    cols
}

fn escape_json(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

fn csv_escape(text: &str) -> String {
    if text.contains(',') || text.contains('"') {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

/// Full audit report for CLI export.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuditReport {
    /// Orphan paths.
    pub orphans: Vec<String>,
    /// Duplicate groups.
    pub duplicates: Vec<DuplicateGroup>,
    /// Size tree root.
    pub size_map: SizeMapNode,
}

/// Runs the offline audit pipeline.
pub fn run_audit(registry: &AssetRegistry, config: &AuditConfig) -> AuditReport {
    AuditReport {
        orphans: find_orphans(registry, config),
        duplicates: find_duplicates(registry),
        size_map: build_size_map(registry),
    }
}

/// Serialises [`AuditReport`] as JSON for cook/CLI.
pub fn export_audit_json(report: &AuditReport) -> String {
    let orphans: Vec<String> = report.orphans.clone();
    let mut out = String::new();
    out.push('{');
    out.push_str("\"orphans\":[");
    for (i, path) in orphans.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(&escape_json(path));
        out.push('"');
    }
    out.push_str("],\"duplicates\":[");
    for (i, group) in report.duplicates.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"hash\":");
        out.push_str(&group.hash.to_string());
        out.push_str(",\"members\":[");
        for (j, (guid, path)) in group.members.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            out.push_str("{\"guid\":\"");
            out.push_str(&guid.to_string());
            out.push_str("\",\"path\":\"");
            out.push_str(&escape_json(path));
            out.push_str("\"}");
        }
        out.push_str("]}");
    }
    out.push_str("],\"size_map\":");
    out.push_str(&export_size_map_json(&report.size_map));
    out.push('}');
    out
}
