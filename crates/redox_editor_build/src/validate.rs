//! Headless project validation for CI and `redox_cook validate`.
//!
//! **Coordinator note (B7):** the canonical entry point is [`validate_project`]. EDT-B7 may
//! later fold scene rules here; until then this checker owns manifest, registry, references
//! and start-scene load diagnostics.

use std::path::Path;

use redox_asset::{AssetRegistry, AssetGuid};
use redox_ecs::World;
use redox_editor_assets::reference_index::{ReferenceIndex, scene_asset_refs};
use redox_editor_core::project::{ProjectContext, ProjectError, ProjectWarning, MANIFEST_FILE};
use redox_editor_scene::{LoadWarning, SceneDocument, register_scene_components};
use redox_reflect::TypeRegistry;
use ron;
use serde::{Deserialize, Serialize};

/// How serious a validation issue is for exit codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValidationSeverity {
    /// Fails the command (exit ≠ 0).
    Error,
    /// Reported but does not fail the command.
    Warn,
}

/// One problem or warning emitted by [`validate_project`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    /// Rule name (for example `MissingAsset`, `DamagedMeta`).
    pub rule: String,
    /// Error or warning.
    pub severity: ValidationSeverity,
    /// File or asset path when known.
    pub path: Option<String>,
    /// Asset GUID when known (32 lowercase hex digits).
    pub guid: Option<String>,
    /// Human-readable explanation.
    pub message: String,
}

/// Full validation output (JSON when `--json` is passed to the CLI).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationReport {
    /// Every issue, stable order: manifest, registry, scenes.
    pub issues: Vec<ValidationIssue>,
}

impl ValidationReport {
    /// Whether any issue has [`ValidationSeverity::Error`].
    pub fn has_errors(&self) -> bool {
        self.issues
            .iter()
            .any(|item| item.severity == ValidationSeverity::Error)
    }

    /// Process exit code: 0 when there are no errors, 1 when there are.
    pub fn exit_code(&self) -> u8 {
        if self.has_errors() { 1 } else { 0 }
    }
}

/// Validates a project under `root`: manifest, assets, scene references and the start scene.
///
/// Opens the project like the editor; fatal manifest errors are returned as `Err`. Everything
/// else is listed in the report. Warnings alone still yield exit code 0.
pub fn validate_project(root: &Path) -> Result<ValidationReport, ProjectError> {
    let project = ProjectContext::open(root)?;
    let mut report = ValidationReport::default();
    for warning in project.warnings() {
        push_project_warning(&mut report, warning);
    }

    let assets_root = root.join(project.manifest().assets_root.as_str());
    if assets_root.is_dir() {
        collect_registry_issues(&mut report, &assets_root);
        collect_reference_issues(&mut report, &project, &assets_root);
    } else {
        report.issues.push(ValidationIssue {
            rule: "MissingAssetsRoot".into(),
            severity: ValidationSeverity::Error,
            path: Some(project.manifest().assets_root.clone()),
            guid: None,
            message: format!(
                "assets root {} does not exist",
                project.manifest().assets_root
            ),
        });
    }

    if let Some(scene) = project.start_scene_path() {
        collect_scene_load_issues(&mut report, &scene, Some(assets_root.as_path()));
    }

    Ok(report)
}

fn push_project_warning(report: &mut ValidationReport, warning: &ProjectWarning) {
    match warning {
        ProjectWarning::MissingPlugin { name } => {
            report.issues.push(ValidationIssue {
                rule: "MissingPlugin".into(),
                severity: ValidationSeverity::Warn,
                path: None,
                guid: None,
                message: format!("plugin {name} is listed but not loaded"),
            });
        }
        ProjectWarning::MissingStartScene { path } => {
            report.issues.push(ValidationIssue {
                rule: "MissingStartScene".into(),
                severity: ValidationSeverity::Error,
                path: Some(relpath_display(path)),
                guid: None,
                message: format!("start scene {} is missing", path.display()),
            });
        }
    }
}

fn collect_registry_issues(report: &mut ValidationReport, assets_root: &Path) {
    let scan = match AssetRegistry::scan(assets_root) {
        Ok((_registry, scan)) => scan,
        Err(err) => {
            report.issues.push(ValidationIssue {
                rule: "AssetScanFailed".into(),
                severity: ValidationSeverity::Error,
                path: Some(relpath_display(assets_root)),
                guid: None,
                message: err.to_string(),
            });
            return;
        }
    };
    for path in scan.damaged {
        report.issues.push(ValidationIssue {
            rule: "DamagedMeta".into(),
            severity: ValidationSeverity::Error,
            path: Some(relpath_display(&path)),
            guid: None,
            message: "sidecar is damaged".into(),
        });
    }
}

fn collect_reference_issues(
    report: &mut ValidationReport,
    project: &ProjectContext,
    assets_root: &Path,
) {
    let Ok((mut registry, _)) = AssetRegistry::scan(assets_root) else {
        return;
    };
    let mut index = ReferenceIndex::default();
    let unreadable = index.rebuild(&mut registry);
    for err in unreadable {
        report.issues.push(ValidationIssue {
            rule: "UnreadableScene".into(),
            severity: ValidationSeverity::Error,
            path: Some(err),
            guid: None,
            message: "scene could not be indexed".into(),
        });
    }
    for miss in index.missing_refs(&registry) {
        report.issues.push(ValidationIssue {
            rule: "MissingAsset".into(),
            severity: ValidationSeverity::Error,
            path: miss.path.clone().or(Some(miss.scene.clone())),
            guid: miss.guid.clone(),
            message: format!(
                "missing asset in {} (component {})",
                miss.scene, miss.component
            ),
        });
    }
    if let Ok(scenes) = project.scenes() {
        for scene in scenes {
            if let Ok(refs) = scene_asset_refs(&scene) {
                for sref in refs {
                    let resolution = redox_editor_assets::resolve_ref(
                        &registry,
                        sref.guid.as_deref(),
                        sref.path.as_deref(),
                    );
                    if matches!(
                        resolution,
                        redox_editor_assets::RefResolution::Missing { .. }
                    ) {
                        report.issues.push(ValidationIssue {
                            rule: "MissingAsset".into(),
                            severity: ValidationSeverity::Error,
                            path: sref.path.clone(),
                            guid: sref.guid.clone(),
                            message: format!(
                                "unresolved reference in {}",
                                relpath_display(&scene)
                            ),
                        });
                    }
                }
            }
        }
    }
}

fn collect_scene_load_issues(
    report: &mut ValidationReport,
    scene_path: &Path,
    assets_root: Option<&Path>,
) {
    let mut registry = TypeRegistry::new();
    register_scene_components(&mut registry);
    let mut world = World::new();
    match SceneDocument::open_with_assets(scene_path, &registry, &mut world, assets_root) {
        Ok((_doc, load)) => push_load_warnings(report, &load),
        Err(err) => {
            report.issues.push(ValidationIssue {
                rule: "SceneLoadFailed".into(),
                severity: ValidationSeverity::Error,
                path: Some(relpath_display(scene_path)),
                guid: None,
                message: err.to_string(),
            });
        }
    }
}

fn push_load_warnings(report: &mut ValidationReport, load: &redox_editor_scene::LoadReport) {
    for warning in &load.warnings {
        match warning {
            LoadWarning::UnknownComponent { entity, type_name } => {
                report.issues.push(ValidationIssue {
                    rule: "UnknownComponent".into(),
                    severity: ValidationSeverity::Warn,
                    path: None,
                    guid: Some(format!("{:032x}", entity.0)),
                    message: format!("unknown component {type_name}"),
                });
            }
            LoadWarning::MissingParent { entity, parent_guid } => {
                report.issues.push(ValidationIssue {
                    rule: "MissingParent".into(),
                    severity: ValidationSeverity::Warn,
                    path: None,
                    guid: Some(format!("{:032x}", entity.0)),
                    message: format!("parent {parent_guid} is missing"),
                });
            }
            LoadWarning::MissingAsset {
                entity,
                type_name,
                path,
            } => {
                report.issues.push(ValidationIssue {
                    rule: "MissingAsset".into(),
                    severity: ValidationSeverity::Error,
                    path: Some(path.clone()),
                    guid: Some(format!("{:032x}", entity.0)),
                    message: format!("{type_name} points at missing {path}"),
                });
            }
            LoadWarning::MissingGuid { entity } => {
                report.issues.push(ValidationIssue {
                    rule: "MissingGuid".into(),
                    severity: ValidationSeverity::Warn,
                    path: None,
                    guid: None,
                    message: format!("entity index {entity} had no guid"),
                });
            }
        }
    }
}

fn relpath_display(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// Writes JSON for automation; schema matches [`ValidationReport`].
pub fn write_validation_json(path: &Path, report: &ValidationReport) -> Result<(), String> {
    let text = serde_json::to_string_pretty(report).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}

/// Parses a manifest without opening the project (tests).
pub fn validate_manifest_only(root: &Path) -> Result<(), ProjectError> {
    let path = root.join(MANIFEST_FILE);
    let text = std::fs::read_to_string(&path).map_err(|source| ProjectError::Io {
        path: path.clone(),
        source,
    })?;
    let _: redox_editor_core::project::ProjectManifest =
        ron::from_str(&text).map_err(|err| ProjectError::Parse {
            path,
            message: err.to_string(),
        })?;
    Ok(())
}

/// Formats a GUID for JSON issues.
pub fn guid_text(guid: AssetGuid) -> String {
    format!("{guid:032x}")
}