//! Slang Shader Engine integration.
//!
//! Provides loading, module resolution, and compilation for `.slang` shaders
//! into SPIR-V and WGSL compatible targets for `wgpu`.
//!
//! Supports modern Slang capabilities:
//! - Modular compilation (`import Common;`)
//! - Interfaces and Generics (`interface IBRDF`)
//! - Parameter blocks (`ParameterBlock<T>`)
//! - Hot-reload tracking and source caching

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::RwLock;

/// Compilation target for Slang shaders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlangTarget {
    Spirv,
    Wgsl,
    Hlsl,
}

impl SlangTarget {
    pub fn as_target_str(&self) -> &'static str {
        match self {
            Self::Spirv => "spirv",
            Self::Wgsl => "wgsl",
            Self::Hlsl => "hlsl",
        }
    }
}

/// Compilation profile for Slang.
#[derive(Clone, Debug)]
pub struct SlangCompileOptions {
    pub target: SlangTarget,
    pub entry_point: String,
    pub stage: &'static str, // "vertex", "fragment", "compute"
    pub search_paths: Vec<PathBuf>,
    pub defines: HashMap<String, String>,
}

impl Default for SlangCompileOptions {
    fn default() -> Self {
        Self {
            target: SlangTarget::Wgsl,
            entry_point: "main".to_string(),
            stage: "compute",
            search_paths: vec![
                PathBuf::from("shaders/slang"),
                PathBuf::from("crates/redox_render/shaders/slang"),
                PathBuf::from("assets/shaders/slang"),
            ],
            defines: HashMap::new(),
        }
    }
}

/// Compilation result containing source code or bytecode.
#[derive(Clone, Debug)]
pub enum SlangOutput {
    Source(String),
    Bytecode(Vec<u8>),
}

/// High-performance compiler and module registry for Slang shaders.
pub struct SlangCompiler {
    search_paths: Vec<PathBuf>,
    cache: RwLock<HashMap<String, SlangOutput>>,
}

impl SlangCompiler {
    /// Creates a new Slang compiler instance.
    pub fn new() -> Self {
        Self {
            search_paths: vec![
                PathBuf::from("shaders/slang"),
                PathBuf::from("crates/redox_render/shaders/slang"),
                PathBuf::from("assets/shaders/slang"),
            ],
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Adds a directory to the module search path.
    pub fn add_search_path(&mut self, path: impl Into<PathBuf>) {
        self.search_paths.push(path.into());
    }

    /// Resolves a Slang file on disk within configured search paths.
    pub fn resolve_file(&self, filename: &str) -> Option<PathBuf> {
        let name_with_ext = if filename.ends_with(".slang") {
            filename.to_string()
        } else {
            format!("{}.slang", filename)
        };

        for base in &self.search_paths {
            let candidate = base.join(&name_with_ext);
            if candidate.exists() {
                return Some(candidate);
            }
        }
        None
    }

    /// Compiles a `.slang` file to the specified target using `slangc`.
    ///
    /// When `slangc` is installed, it runs native cross-compilation with optimization.
    /// Returns compiled shader code or an actionable error message.
    pub fn compile_file(
        &self,
        file_path: &Path,
        options: &SlangCompileOptions,
    ) -> Result<SlangOutput, String> {
        let cache_key = format!(
            "{}:{}:{}:{:?}",
            file_path.display(),
            options.entry_point,
            options.stage,
            options.target
        );

        if let Ok(cache) = self.cache.read() {
            if let Some(cached) = cache.get(&cache_key) {
                return Ok(cached.clone());
            }
        }

        let mut cmd = Command::new("slangc");
        cmd.arg(file_path);
        cmd.arg("-entry").arg(&options.entry_point);
        cmd.arg("-stage").arg(options.stage);
        cmd.arg("-target").arg(options.target.as_target_str());

        // Add search paths
        for path in &self.search_paths {
            cmd.arg("-I").arg(path);
        }
        for path in &options.search_paths {
            cmd.arg("-I").arg(path);
        }

        // Add defines
        for (k, v) in &options.defines {
            cmd.arg(format!("-D{}={}", k, v));
        }

        match cmd.output() {
            Ok(output) if output.status.success() => {
                let result = match options.target {
                    SlangTarget::Spirv => SlangOutput::Bytecode(output.stdout),
                    SlangTarget::Wgsl | SlangTarget::Hlsl => {
                        let text = String::from_utf8_lossy(&output.stdout).to_string();
                        SlangOutput::Source(text)
                    }
                };

                if let Ok(mut cache) = self.cache.write() {
                    cache.insert(cache_key, result.clone());
                }
                Ok(result)
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(format!("slangc compilation failed: {}", stderr))
            }
            Err(e) => {
                // Return descriptive error when slangc CLI is not available in PATH
                Err(format!(
                    "slangc not found on PATH ({}). Ensure Slang SDK is installed to compile .slang shaders natively.",
                    e
                ))
            }
        }
    }

    /// Clears the compilation cache (for hot-reload).
    pub fn invalidate_cache(&self) {
        if let Ok(mut cache) = self.cache.write() {
            cache.clear();
        }
    }
}

impl Default for SlangCompiler {
    fn default() -> Self {
        Self::new()
    }
}
