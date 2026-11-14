//! Validation issues, severities and the aggregated report.

use redox_scene::EntityGuid;
use serde::{Deserialize, Serialize};

/// Identifier of a validation rule (stable for CLI and filters).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ValidationRule {
    /// A handle path does not exist under the assets root.
    MissingAsset,
    /// An asset reference has a broken GUID or inconsistent path hint.
    BrokenAssetRef,
    /// A component type in the scene is not registered.
    UnknownComponent,
    /// A parent guid from the file was missing at load time.
    MissingParent,
    /// The entity hierarchy contains a cycle.
    HierarchyCycle,
    /// No entity carries a camera and none is marked active.
    NoActiveCamera,
    /// Material graph failed to compile (conditional; may be skipped).
    MaterialCompileError,
    /// A large texture has no mip chain configured.
    TextureNoMips,
    /// An entity transform lies outside the configured world bounds.
    OutOfWorldBounds,
}

impl ValidationRule {
    /// Short stable name for logs and JSON.
    pub fn id(self) -> &'static str {
        match self {
            Self::MissingAsset => "missing_asset",
            Self::BrokenAssetRef => "broken_asset_ref",
            Self::UnknownComponent => "unknown_component",
            Self::MissingParent => "missing_parent",
            Self::HierarchyCycle => "hierarchy_cycle",
            Self::NoActiveCamera => "no_active_camera",
            Self::MaterialCompileError => "material_compile_error",
            Self::TextureNoMips => "texture_no_mips",
            Self::OutOfWorldBounds => "out_of_world_bounds",
        }
    }
}

/// How severe a validation issue is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Severity {
    /// Informational; the scene still runs.
    Info,
    /// Something is wrong but not blocking by default.
    Warning,
    /// Must be fixed before shipping; may block PIE when configured.
    Error,
}

/// A fix the editor can apply in one undo step when safe.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationFix {
    /// Spawn an entity with a default [`redox_render::Camera`].
    AddDefaultCamera,
    /// Detach `entity` from its parent to break a hierarchy cycle.
    Unparent {
        /// Entity to detach.
        entity: EntityGuid,
    },
}

/// One problem found by a rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    /// Rule that produced the issue.
    pub rule: ValidationRule,
    /// Severity assigned by the rule.
    pub severity: Severity,
    /// Human-readable explanation.
    pub message: String,
    /// Entity the issue refers to, when any.
    pub entity: Option<EntityGuid>,
    /// Asset path relative to the assets root, when any.
    pub asset_path: Option<String>,
    /// Scene file path, when validating a project (relative to project root).
    pub scene_path: Option<String>,
    /// Optional automatic fix.
    pub fix: Option<ValidationFix>,
    /// Entity to select when the user chooses Select.
    pub select: Option<EntityGuid>,
    /// Asset or scene document to open when the user chooses Open.
    pub open: Option<String>,
}

impl ValidationIssue {
    /// Whether the issue is an error.
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// Everything a validation pass reported.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationReport {
    /// Issues in rule order, then severity.
    pub issues: Vec<ValidationIssue>,
}

impl ValidationReport {
    /// Whether any issue has [`Severity::Error`].
    pub fn has_errors(&self) -> bool {
        self.issues.iter().any(|issue| issue.is_error())
    }

    /// Whether any issue is warning or error.
    pub fn has_problems(&self) -> bool {
        !self.issues.is_empty()
    }

    /// Merges another report into this one.
    pub fn extend(&mut self, other: ValidationReport) {
        self.issues.extend(other.issues);
    }

    /// Issues at or above `min` severity.
    pub fn filtered(&self, min: Severity) -> Vec<&ValidationIssue> {
        self.issues
            .iter()
            .filter(|issue| issue.severity >= min)
            .collect()
    }
}
