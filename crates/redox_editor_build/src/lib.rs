//! Build profiles, dependency-aware cook and project validation for RedOx Studio (EDT-B8/C1).

#![warn(missing_docs)]

mod cook;
mod plan;
mod service;
mod snapshot;
mod validate;

pub use cook::{CookAssetEntry, CookReport, cook_pack};
pub use plan::{build_cook_plan, seed_scene_path, CookPlan, InclusionReason};
pub use service::{
    BuildError, BuildService, list_profiles, resolve_profile, resolve_profile_for_cook,
};
pub use snapshot::BuildSnapshot;
pub use validate::{
    ValidationIssue, ValidationReport, ValidationSeverity, guid_text, validate_manifest_only,
    validate_project, write_validation_json,
};

/// Placeholder kept for older imports; use [`BuildService`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BuildServiceStub;

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;