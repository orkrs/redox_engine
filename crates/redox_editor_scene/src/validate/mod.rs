//! Scene validation rules for the open level and for whole projects (EDT-B7).

pub mod actions;
pub mod issue;
mod project;
pub mod rules;
pub mod settings;

pub use actions::{
    apply_fix, apply_issue_action, IssueUserAction, ValidationActionResult,
};
pub use issue::{
    Severity, ValidationFix, ValidationIssue, ValidationReport, ValidationRule,
};
pub use project::{validate_project, ValidateProjectError};
pub use rules::RuleContext;
pub use settings::ValidationSettings;

use redox_ecs::World;
use redox_reflect::TypeRegistry;
use std::path::Path;

use crate::LoadReport;

/// Options for [`validate_level`].
#[derive(Clone, Debug, Default)]
pub struct ValidateLevelOptions<'a> {
    /// Assets folder for path checks.
    pub assets_root: Option<&'a Path>,
    /// Asset registry when validating with the asset service.
    pub asset_registry: Option<&'a redox_asset::AssetRegistry>,
    /// Parsed scene references.
    pub reference_index: Option<&'a redox_editor_assets::ReferenceIndex>,
    /// Scene path relative to the project root, for issue metadata.
    pub scene_path: Option<&'a str>,
    /// Bounds, texture threshold and editor hooks.
    pub settings: ValidationSettings,
    /// Load warnings to map into validation issues.
    pub load_report: Option<&'a LoadReport>,
}

impl<'a> ValidateLevelOptions<'a> {
    /// Convenience builder with only the assets root set.
    pub fn with_assets_root(root: &'a Path) -> Self {
        Self {
            assets_root: Some(root),
            ..Self::default()
        }
    }
}

/// Runs every v1 rule on the open `world` and returns a report.
///
/// Rule 5 (material graph compile) is skipped until a headless compiler hook exists (📌).
pub fn validate_level(
    world: &World,
    registry: &TypeRegistry,
    options: ValidateLevelOptions<'_>,
) -> ValidationReport {
    let settings = options.settings.clamped();
    let ctx = rules::RuleContext {
        world,
        registry,
        assets_root: options.assets_root,
        asset_registry: options.asset_registry,
        reference_index: options.reference_index,
        scene_path: options.scene_path,
        settings: &settings,
        load_report: options.load_report,
    };
    let mut report = ValidationReport::default();
    report.issues.extend(rules::check_missing_and_broken_assets(&ctx));
    report.issues.extend(rules::check_unknown_components(&ctx));
    report.issues.extend(rules::check_hierarchy(&ctx));
    report.issues.extend(rules::check_active_camera(&ctx));
    report.issues.extend(rules::check_material_compile(&ctx));
    report.issues.extend(rules::check_texture_mips(&ctx));
    report.issues.extend(rules::check_world_bounds(&ctx));
    report
}

/// Whether play-in-editor should be blocked for this report and settings.
pub fn blocks_pie(report: &ValidationReport, settings: &ValidationSettings) -> bool {
    settings.block_pie_on_error && report.has_errors()
}