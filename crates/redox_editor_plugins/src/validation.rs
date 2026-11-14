//! Validation rules plugins may register (scene checks; full engine comes with EDT-B7).

/// How serious an issue is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ValidationSeverity {
    /// Informational note.
    Info,
    /// Something to review.
    Warning,
    /// Blocks export or play until fixed.
    Error,
}

/// One issue produced by a validation rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationIssue {
    /// Rule that reported the issue.
    pub rule_id: String,
    /// Short message for the Issues panel.
    pub message: String,
    /// Severity for filtering.
    pub severity: ValidationSeverity,
}

/// A plugin-provided validation rule (invoked by the future B7 runner).
pub trait EditorValidationRule: Send + Sync {
    /// Stable rule identifier.
    fn id(&self) -> &str;
    /// Human-readable title.
    fn title(&self) -> &str;
    /// Runs the rule; an empty vector means no issues.
    fn check(&self) -> Vec<ValidationIssue>;
}
