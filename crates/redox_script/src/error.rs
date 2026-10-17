//! Error type of the script runtime.

/// Everything that can go wrong when loading or running a script.
#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    /// The bytes are not a valid WebAssembly module.
    #[error("module failed to compile: {0}")]
    Compile(String),
    /// Instantiation failed: unknown or unpermitted import, memory over the limit, bad start.
    #[error("module failed to link: {0}")]
    Link(String),
    /// A required or requested export does not exist (or has the wrong signature).
    #[error("missing export `{0}`")]
    MissingExport(String),
    /// The script trapped; the instance is disabled.
    #[error("script trapped: {0}")]
    Trap(String),
    /// The script exhausted its fuel budget; the instance is disabled.
    #[error("script ran out of fuel")]
    OutOfFuel,
    /// The instance was disabled by an earlier failure (or the id is unknown).
    #[error("script instance is disabled")]
    Disabled,
    /// Reading the module from an asset source failed.
    #[error("asset error: {0}")]
    Asset(String),
}
