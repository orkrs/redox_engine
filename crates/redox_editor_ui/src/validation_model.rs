//! Headless Issues panel model (EDT-B7 step 3): filtering and actions without egui.

use redox_editor_assets::{AssetRegistry, ReferenceIndex};
use redox_editor_scene::validate::{
    apply_issue_action, validate_level, IssueUserAction, Severity, ValidateLevelOptions,
    ValidationIssue, ValidationReport, ValidationSettings,
};
use redox_editor_scene::{Level, LoadReport};
use redox_ecs::World;
use redox_reflect::TypeRegistry;
use std::path::Path;

/// Severity filter for the issues list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SeverityFilter {
    /// Show every issue.
    #[default]
    All,
    /// Warnings and errors only.
    WarnPlus,
    /// Errors only.
    Errors,
}

impl SeverityFilter {
    /// Whether `severity` passes this filter.
    pub fn allows(self, severity: Severity) -> bool {
        match self {
            Self::All => true,
            Self::WarnPlus => severity >= Severity::Warning,
            Self::Errors => severity == Severity::Error,
        }
    }
}

/// User action produced by the panel (Select / Open / Fix).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationPanelAction {
    /// Focus an entity.
    Select(redox_scene::EntityGuid),
    /// Open a document path in the asset browser.
    Open(String),
    /// A fix was applied.
    Fixed,
    /// The action had no effect.
    None,
}

/// State of the Issues tab without egui.
#[derive(Clone, Debug, Default)]
pub struct ValidationPanelModel {
    /// Filter applied to [`Self::report`].
    pub filter: SeverityFilter,
    /// Result of the last validation pass.
    pub report: ValidationReport,
    /// Bounds and editor policy.
    pub settings: ValidationSettings,
    /// Load warnings of the current level, fed into validation.
    pub load_report: LoadReport,
}

impl ValidationPanelModel {
    /// Runs validation on the open level and stores the report.
    pub fn validate_open_level(
        &mut self,
        world: &World,
        registry: &TypeRegistry,
        level: &Level,
        assets_root: Option<&Path>,
        asset_registry: Option<&AssetRegistry>,
        reference_index: Option<&ReferenceIndex>,
    ) {
        let scene_path = level.path().and_then(|path| path.to_str());
        self.report = validate_level(
            world,
            registry,
            ValidateLevelOptions {
                assets_root,
                asset_registry,
                reference_index,
                scene_path,
                settings: self.settings,
                load_report: Some(&self.load_report),
            },
        );
    }

    /// Issues that pass the current severity filter.
    pub fn filtered_issues(&self) -> Vec<&ValidationIssue> {
        self.report
            .issues
            .iter()
            .filter(|issue| self.filter.allows(issue.severity))
            .collect()
    }

    /// Applies an issue action and returns what the host should do (selection, open path).
    pub fn apply_action(
        &self,
        issue: &ValidationIssue,
        action: IssueUserAction,
        world: &mut World,
        registry: &TypeRegistry,
        undo: &mut redox_editor_core::UndoStack,
    ) -> Result<ValidationPanelAction, redox_editor_core::UndoError> {
        use redox_editor_scene::validate::ValidationActionResult;
        match apply_issue_action(issue, action, world, registry, undo)? {
            ValidationActionResult::Select(guid) => Ok(ValidationPanelAction::Select(guid)),
            ValidationActionResult::Open(path) => Ok(ValidationPanelAction::Open(path)),
            ValidationActionResult::Fixed => Ok(ValidationPanelAction::Fixed),
            ValidationActionResult::Noop => Ok(ValidationPanelAction::None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_editor_scene::register_scene_components;
    use redox_scene::EntityGuid;

    #[test]
    fn severity_filter_keeps_only_errors_when_asked() {
        let mut model = ValidationPanelModel::default();
        model.report.issues = vec![
            ValidationIssue {
                rule: redox_editor_scene::validate::ValidationRule::NoActiveCamera,
                severity: Severity::Error,
                message: "x".into(),
                entity: None,
                asset_path: None,
                scene_path: None,
                fix: None,
                select: None,
                open: None,
            },
            ValidationIssue {
                rule: redox_editor_scene::validate::ValidationRule::OutOfWorldBounds,
                severity: Severity::Warning,
                message: "y".into(),
                entity: Some(EntityGuid::new_random()),
                asset_path: None,
                scene_path: None,
                fix: None,
                select: None,
                open: None,
            },
        ];
        model.filter = SeverityFilter::Errors;
        assert_eq!(model.filtered_issues().len(), 1);
        assert_eq!(model.filtered_issues()[0].severity, Severity::Error);
    }

    #[test]
    fn validate_open_level_finds_a_missing_camera() {
        let mut registry = TypeRegistry::new();
        register_scene_components(&mut registry);
        let world = World::new();
        let level = redox_editor_scene::Level::new_scene("test", &registry, &world);
        let mut model = ValidationPanelModel::default();
        model.validate_open_level(&world, &registry, &level, None, None, None);
        assert!(model.report.has_errors());
    }
}